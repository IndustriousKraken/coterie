//! Domain operations for recording a Coterie-side payment.
//!
//! Stripe-driven flows (Checkout sessions, saved-card charges) live in
//! `payments::StripeClient` because they're outbound; this service is
//! the Coterie-side counterpart for the "an admin says this payment
//! happened" path. It owns the validation + persist + dues-extension +
//! audit-log chain that was duplicated across three handlers
//! (`admin_record_payment_submit`, `create_manual`, `waive`) before
//! consolidation.
//!
//! Donation Stripe-checkout flows are NOT covered here — those build a
//! Pending row through `StripeClient::create_*_checkout_session` and
//! flip to Completed via the webhook dispatcher. A different shape, a
//! different code path.

use std::sync::Arc;
use uuid::Uuid;

use crate::{
    domain::{Payer, Payment, PaymentKind, PaymentMethod, PaymentStatus, MAX_PAYMENT_CENTS},
    error::{AppError, Result},
    repository::{DonationCampaignRepository, MemberRepository, PaymentRepository},
    service::{audit_service::AuditService, billing_service::BillingService},
};

/// Input for `PaymentService::record_manual`. The wire-format parsing
/// (form vs. JSON) stays in handlers; the service receives the typed
/// shape. Putting validation here means the campaign-existence check
/// and the amount cap are enforced uniformly regardless of caller.
pub struct RecordManualPaymentInput {
    pub member_id: Uuid,
    pub amount_cents: i64,
    pub kind: PaymentKind,
    pub description: String,
    /// `Manual` for normal admin records, `Waived` for $0 dues
    /// waivers. `Stripe` is rejected — that path goes through
    /// `StripeClient`, not here.
    pub payment_method: PaymentMethod,
    /// When `kind` is `Membership` and this is `Some`, the service
    /// runs `BillingService::extend_member_dues_by_slug` and
    /// `reschedule_after_payment` against this slug. Donations and
    /// `Other` ignore this field.
    pub membership_type_slug: Option<String>,
    /// The admin's member id, for audit logging.
    pub actor_id: Uuid,
}

pub struct PaymentService {
    payment_repo: Arc<dyn PaymentRepository>,
    member_repo: Arc<dyn MemberRepository>,
    donation_campaign_repo: Arc<dyn DonationCampaignRepository>,
    audit_service: Arc<AuditService>,
}

impl PaymentService {
    pub fn new(
        payment_repo: Arc<dyn PaymentRepository>,
        member_repo: Arc<dyn MemberRepository>,
        donation_campaign_repo: Arc<dyn DonationCampaignRepository>,
        audit_service: Arc<AuditService>,
    ) -> Self {
        Self { payment_repo, member_repo, donation_campaign_repo, audit_service }
    }

    /// Record a manual or waived payment, optionally extending dues.
    ///
    /// Validates: amount within `[0, MAX_PAYMENT_CENTS]`, member
    /// exists, donation campaign (if supplied) exists, payment_method
    /// is not Stripe. Persists the row, then if `kind` is Membership
    /// and a slug was supplied, extends dues and reschedules the next
    /// auto-renew via `billing_service`. Failures in the post-work
    /// chain are logged but don't roll back the payment row — same
    /// semantics as the original handlers.
    pub async fn record_manual(
        &self,
        input: RecordManualPaymentInput,
        billing_service: &BillingService,
    ) -> Result<Payment> {
        // ---- Validation -----------------------------------------------
        if input.amount_cents < 0 {
            return Err(AppError::BadRequest(
                "amount_cents must not be negative".to_string(),
            ));
        }
        if input.amount_cents > MAX_PAYMENT_CENTS {
            return Err(AppError::BadRequest(format!(
                "amount_cents exceeds the ${} cap on a single payment",
                MAX_PAYMENT_CENTS / 100,
            )));
        }
        if matches!(input.payment_method, PaymentMethod::Stripe) {
            return Err(AppError::BadRequest(
                "Stripe payments are recorded via StripeClient, not record_manual".to_string(),
            ));
        }
        if self.member_repo.find_by_id(input.member_id).await?.is_none() {
            return Err(AppError::BadRequest(format!(
                "member {} not found",
                input.member_id,
            )));
        }
        // Campaign-existence check: the dropdown only offers valid
        // campaigns, but a stale form / forged JSON could otherwise
        // create an orphan donation row.
        if let PaymentKind::Donation { campaign_id: Some(cid) } = input.kind {
            if self.donation_campaign_repo.find_by_id(cid).await?.is_none() {
                return Err(AppError::BadRequest(
                    "donation_campaign_id doesn't match any campaign".to_string(),
                ));
            }
        }

        // ---- Persist --------------------------------------------------
        let now = chrono::Utc::now();
        let payment = Payment {
            id: Uuid::new_v4(),
            payer: Payer::Member(input.member_id),
            amount_cents: input.amount_cents,
            currency: "USD".to_string(),
            status: PaymentStatus::Completed,
            payment_method: input.payment_method.clone(),
            external_id: None,
            description: input.description.clone(),
            kind: input.kind.clone(),
            paid_at: Some(now),
            created_at: now,
            updated_at: now,
        };
        let payment = self.payment_repo.create(payment).await?;

        // ---- Post-work: dues + reschedule (membership only) ----------
        if matches!(input.kind, PaymentKind::Membership) {
            if let Some(slug) = &input.membership_type_slug {
                if let Err(e) = billing_service
                    .extend_member_dues_by_slug(payment.id, input.member_id, slug)
                    .await
                {
                    tracing::error!(
                        "Recorded {:?} payment for {} but dues extension failed: {}",
                        input.payment_method, input.member_id, e,
                    );
                }
                if let Err(e) = billing_service
                    .reschedule_after_payment(input.member_id, slug)
                    .await
                {
                    tracing::error!(
                        "Recorded {:?} payment for {} but reschedule failed: {}",
                        input.payment_method, input.member_id, e,
                    );
                }
            }
        }

        // ---- Audit ----------------------------------------------------
        let action = audit_action(&input.payment_method, &input.kind);
        self.audit_service.log(
            Some(input.actor_id),
            action,
            "member",
            &input.member_id.to_string(),
            None,
            Some(&format!(
                "${:.2} — {}",
                input.amount_cents as f64 / 100.0,
                input.description,
            )),
            None,
        ).await;

        Ok(payment)
    }
}

/// Audit action string for the recorded payment. Centralized so the
/// four sites that used to duplicate this can't drift.
fn audit_action(method: &PaymentMethod, kind: &PaymentKind) -> &'static str {
    match (method, kind) {
        (PaymentMethod::Waived, _) => "waive_dues",
        (_, PaymentKind::Membership) => "manual_payment",
        (_, PaymentKind::Donation { .. }) => "manual_donation",
        (_, PaymentKind::Other) => "manual_other",
    }
}
