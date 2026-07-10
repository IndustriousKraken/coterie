//! One-time, idempotent, INSERT-only backfill of members' historical
//! Stripe charges and saved cards into Coterie.
//!
//! This is the **fourth payment-recording entry point** (see the
//! `payment-recording` capability spec). Like the member CSV import it:
//!   - persists payments via `payment_repo.create`, keyed on the Stripe
//!     id so a re-run creates nothing new,
//!   - emits its OWN audit rows (`import_payment` per created payment,
//!     one `import_payments_batch` aggregate per run) rather than
//!     routing through the other three sites, and
//!   - records only already-settled history: it never initiates a
//!     charge, extends dues, dispatches integration events, or sends a
//!     receipt email.
//!
//! Saved-card import mirrors cards already attached to the member's
//! Stripe customer (no SetupIntent, no raw card numbers), de-duplicated
//! by Stripe card fingerprint, and sets the member's default from the
//! Stripe customer's default payment method.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    domain::{Payer, Payment, PaymentKind, PaymentMethod, PaymentStatus, SavedCard, StripeRef},
    error::Result,
    payments::gateway::{ChargeSummary, StripeGateway},
    repository::{MemberRepository, PaymentRepository, SavedCardRepository},
    service::audit_service::AuditService,
};

/// Tally returned to the admin UI after a run.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct StripeImportSummary {
    /// Payment rows created this run.
    pub payments_imported: u32,
    /// Charges already present (idempotent re-run, or a live-recorded
    /// row with the same Stripe id).
    pub payments_skipped: u32,
    /// Card rows created this run.
    pub cards_imported: u32,
    /// Cards skipped as duplicates (fingerprint or pm id already saved).
    pub cards_skipped: u32,
}

pub struct StripeImportService {
    gateway: Arc<dyn StripeGateway>,
    payment_repo: Arc<dyn PaymentRepository>,
    saved_card_repo: Arc<dyn SavedCardRepository>,
    member_repo: Arc<dyn MemberRepository>,
    audit_service: Arc<AuditService>,
}

impl StripeImportService {
    pub fn new(
        gateway: Arc<dyn StripeGateway>,
        payment_repo: Arc<dyn PaymentRepository>,
        saved_card_repo: Arc<dyn SavedCardRepository>,
        member_repo: Arc<dyn MemberRepository>,
        audit_service: Arc<AuditService>,
    ) -> Self {
        Self {
            gateway,
            payment_repo,
            saved_card_repo,
            member_repo,
            audit_service,
        }
    }

    /// Backfill charges + cards for every member with a Stripe customer.
    /// `actor_id` is the admin who triggered the run (recorded on the
    /// audit rows). Per-member failures are logged and skipped so one
    /// bad customer doesn't abort the whole import; the run always
    /// emits its `import_payments_batch` aggregate.
    pub async fn backfill_all(&self, actor_id: Uuid) -> Result<StripeImportSummary> {
        let members = self.member_repo.list_with_stripe_customer_id().await?;
        let mut summary = StripeImportSummary::default();

        for member in &members {
            let customer_id = match member.stripe_customer_id.as_deref() {
                Some(c) if !c.is_empty() => c,
                _ => continue,
            };

            match self
                .import_member_charges(actor_id, member.id, customer_id)
                .await
            {
                Ok((imported, skipped)) => {
                    summary.payments_imported += imported;
                    summary.payments_skipped += skipped;
                }
                Err(e) => tracing::error!(
                    "Stripe payment backfill failed for member {} ({}): {}",
                    member.id,
                    customer_id,
                    e
                ),
            }

            match self.import_member_cards(member.id, customer_id).await {
                Ok((imported, skipped)) => {
                    summary.cards_imported += imported;
                    summary.cards_skipped += skipped;
                }
                Err(e) => tracing::error!(
                    "Stripe card backfill failed for member {} ({}): {}",
                    member.id,
                    customer_id,
                    e
                ),
            }
        }

        // One aggregate audit row for the run — mirrors the member CSV
        // import's `import_members_batch`.
        let summary_str = format!(
            "members={},payments_imported={},payments_skipped={},cards_imported={},cards_skipped={}",
            members.len(),
            summary.payments_imported,
            summary.payments_skipped,
            summary.cards_imported,
            summary.cards_skipped,
        );
        self.audit_service
            .log(
                Some(actor_id),
                "import_payments_batch",
                "payment",
                "*",
                None,
                Some(&summary_str),
                None,
            )
            .await;

        Ok(summary)
    }

    /// Import one customer's settled historical charges. Returns
    /// (imported, skipped).
    async fn import_member_charges(
        &self,
        actor_id: Uuid,
        member_id: Uuid,
        customer_id: &str,
    ) -> Result<(u32, u32)> {
        let charges = self.gateway.list_charges(customer_id).await?;
        let mut imported = 0u32;
        let mut skipped = 0u32;

        for charge in charges {
            // Only already-settled charges become receipts. Pending
            // never landed; failed didn't land.
            if charge.status != "succeeded" {
                continue;
            }

            // Key on the invoice id first (matches what the subscription
            // invoice webhook stores), then the payment-intent id
            // (matches the auto-renew charge path). Keying on the same
            // id the live paths use makes the backfill de-dup against
            // payments Coterie already recorded, not just against itself.
            let stripe_ref = match (&charge.invoice_id, &charge.payment_intent_id) {
                (Some(inv), _) if !inv.is_empty() => StripeRef::Invoice(inv.clone()),
                (_, Some(pi)) if !pi.is_empty() => StripeRef::PaymentIntent(pi.clone()),
                // No id we can key on — skip rather than risk a row we
                // can't make idempotent.
                _ => continue,
            };
            let stripe_key = stripe_ref.as_str().to_string();

            // Idempotency anchor: the partial-unique index on
            // stripe_payment_id. Check first so a re-run (or a live dup)
            // is a clean skip rather than a DB unique-violation error.
            if self
                .payment_repo
                .find_by_stripe_id(&stripe_key)
                .await?
                .is_some()
            {
                skipped += 1;
                continue;
            }

            let description = charge
                .description
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "Imported Stripe payment".to_string());
            // Historical "when paid" — preserved so receipts and the
            // annual statement group by the real charge year. (create
            // stamps created_at = now, but honors paid_at.)
            let paid_at: Option<DateTime<Utc>> = DateTime::from_timestamp(charge.created, 0);
            let now = Utc::now();

            let payment = Payment {
                id: Uuid::new_v4(),
                payer: Payer::Member(member_id),
                amount_cents: charge.amount_cents,
                currency: charge.currency.clone(),
                status: PaymentStatus::Completed,
                payment_method: PaymentMethod::Stripe,
                kind: classify_kind(&charge),
                external_id: Some(stripe_ref),
                description,
                paid_at,
                created_at: now,
                updated_at: now,
            };

            let payment = self.payment_repo.create(payment).await?;
            imported += 1;

            // Self-audit: one row per created payment. NO dues
            // extension, NO integration events, NO receipt email.
            self.audit_service
                .log(
                    Some(actor_id),
                    "import_payment",
                    "payment",
                    &payment.id.to_string(),
                    None,
                    Some(&format!(
                        "${:.2} — {} ({})",
                        payment.amount_cents as f64 / 100.0,
                        payment.description,
                        stripe_key,
                    )),
                    None,
                )
                .await;
        }

        Ok((imported, skipped))
    }

    /// Mirror one customer's attached cards into `payment_methods`,
    /// de-duplicated by fingerprint (falling back to pm id), and set the
    /// member's default from the Stripe customer's default payment
    /// method. Returns (imported, skipped).
    async fn import_member_cards(&self, member_id: Uuid, customer_id: &str) -> Result<(u32, u32)> {
        let cards = self.gateway.list_payment_methods(customer_id).await?;

        let existing = self.saved_card_repo.find_by_member(member_id).await?;
        // Track fingerprints seen so far (existing rows + ones imported
        // this run) so two Stripe pms sharing a fingerprint don't both
        // land.
        let mut seen_fingerprints: Vec<String> = existing
            .iter()
            .filter_map(|c| c.fingerprint.clone())
            .collect();
        let existing_pm_ids: Vec<String> = existing
            .iter()
            .map(|c| c.stripe_payment_method_id.clone())
            .collect();

        let mut imported = 0u32;
        let mut skipped = 0u32;
        let now = Utc::now();

        for card in cards {
            let dup_by_fingerprint = card
                .fingerprint
                .as_ref()
                .is_some_and(|fp| seen_fingerprints.iter().any(|s| s == fp));
            let dup_by_pm = existing_pm_ids.iter().any(|p| p == &card.id);
            if dup_by_fingerprint || dup_by_pm {
                skipped += 1;
                continue;
            }

            let saved = SavedCard {
                id: Uuid::new_v4(),
                member_id,
                stripe_payment_method_id: card.id.clone(),
                card_last_four: card.last4.clone(),
                card_brand: card.brand.clone(),
                exp_month: card.exp_month,
                exp_year: card.exp_year,
                // Import with is_default=false; the correct default is
                // set from Stripe's default_payment_method below.
                is_default: false,
                fingerprint: card.fingerprint.clone(),
                created_at: now,
                updated_at: now,
            };
            self.saved_card_repo.create(saved).await?;
            imported += 1;
            if let Some(fp) = card.fingerprint {
                seen_fingerprints.push(fp);
            }
        }

        // Set the member's default from Stripe's default payment method,
        // so a member who later converts to Coterie-managed keeps being
        // charged on the same card Stripe was already using — not
        // whichever card happened to import first.
        if imported > 0 {
            if let Ok(customer) = self.gateway.retrieve_customer(customer_id).await {
                if let Some(default_pm) = customer.default_payment_method_id {
                    let all = self.saved_card_repo.find_by_member(member_id).await?;
                    if let Some(default_card) = all
                        .iter()
                        .find(|c| c.stripe_payment_method_id == default_pm)
                    {
                        self.saved_card_repo
                            .set_default(member_id, default_card.id)
                            .await?;
                    }
                }
            }
        }

        Ok((imported, skipped))
    }
}

/// Map a Stripe charge to a Coterie payment kind. Honors an explicit
/// `payment_type` metadata hint where present; otherwise defaults to
/// membership dues (the common case for subscription invoices).
fn classify_kind(charge: &ChargeSummary) -> PaymentKind {
    match charge.metadata.get("payment_type").map(|s| s.as_str()) {
        Some("donation") => PaymentKind::Donation { campaign_id: None },
        Some("other") => PaymentKind::Other,
        _ => PaymentKind::Membership,
    }
}
