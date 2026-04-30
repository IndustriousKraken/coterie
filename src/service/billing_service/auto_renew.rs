//! Auto-renew lifecycle + the scheduled-payment charge runner. The
//! two halves share `schedule_renewal` and the same idempotency
//! invariants (cancel-pending-first, atomic per-payment dues claim,
//! local-state-flips-before-Stripe-cancel rollback ordering), so
//! they live together rather than splitting further.
//!
//! Doesn't own notifications — auto-renew failure paths fire
//! `IntegrationEvent::AdminAlert` directly via the integration_manager.
//! The member-facing card-declined notice goes through `Notifications`
//! (driven from `WebhookDispatcher`), not from here.

use chrono::Utc;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    domain::{
        configurable_types::BillingPeriod, BillingMode, Payment, PaymentMethod, PaymentStatus,
        PaymentType, SavedCard, ScheduledPayment, ScheduledPaymentStatus,
    },
    error::{AppError, Result},
    integrations::{IntegrationEvent, IntegrationManager},
    payments::StripeClient,
    repository::{
        MemberRepository, PaymentRepository, SavedCardRepository, ScheduledPaymentRepository,
    },
    service::{membership_type_service::MembershipTypeService, settings_service::SettingsService},
};

/// Result of `bulk_migrate_stripe_subscriptions` — per-batch summary
/// the admin UI displays after running the migration.
#[derive(Debug, Default)]
pub struct BulkMigrationSummary {
    pub succeeded: u32,
    pub skipped: u32,
    pub failed: Vec<(Uuid, String)>,
}

pub struct AutoRenew {
    scheduled_payment_repo: Arc<dyn ScheduledPaymentRepository>,
    payment_repo: Arc<dyn PaymentRepository>,
    saved_card_repo: Arc<dyn SavedCardRepository>,
    member_repo: Arc<dyn MemberRepository>,
    membership_type_service: Arc<MembershipTypeService>,
    settings_service: Arc<SettingsService>,
    integration_manager: Arc<IntegrationManager>,
    stripe_client: Option<Arc<StripeClient>>,
}

impl AutoRenew {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        scheduled_payment_repo: Arc<dyn ScheduledPaymentRepository>,
        payment_repo: Arc<dyn PaymentRepository>,
        saved_card_repo: Arc<dyn SavedCardRepository>,
        member_repo: Arc<dyn MemberRepository>,
        membership_type_service: Arc<MembershipTypeService>,
        settings_service: Arc<SettingsService>,
        integration_manager: Arc<IntegrationManager>,
        stripe_client: Option<Arc<StripeClient>>,
    ) -> Self {
        Self {
            scheduled_payment_repo,
            payment_repo,
            saved_card_repo,
            member_repo,
            membership_type_service,
            settings_service,
            integration_manager,
            stripe_client,
        }
    }

    /// Schedule a renewal payment for a member based on their membership type.
    /// Called after a successful payment to schedule the next one.
    pub async fn schedule_renewal(
        &self,
        member_id: Uuid,
        membership_type_slug: &str,
    ) -> Result<ScheduledPayment> {
        let membership_type = self
            .membership_type_service
            .get_by_slug(membership_type_slug)
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "Membership type '{}' not found",
                    membership_type_slug
                ))
            })?;

        let billing_period = membership_type
            .billing_period_enum()
            .unwrap_or(BillingPeriod::Yearly);

        // Don't schedule renewals for lifetime memberships
        if billing_period == BillingPeriod::Lifetime {
            return Err(AppError::BadRequest(
                "Cannot schedule renewal for lifetime membership".to_string(),
            ));
        }

        // Get current dues_paid_until to determine next due date.
        let next_due = match self.member_repo.find_by_id(member_id).await? {
            Some(m) => m.dues_paid_until
                .map(|d| d.date_naive())
                .unwrap_or_else(|| Utc::now().date_naive()),
            None => Utc::now().date_naive(),
        };

        let membership_type_id = membership_type.id;

        let scheduled = ScheduledPayment {
            id: Uuid::new_v4(),
            member_id,
            membership_type_id,
            amount_cents: membership_type.fee_cents as i64,
            currency: "USD".to_string(),
            due_date: next_due,
            status: ScheduledPaymentStatus::Pending,
            retry_count: 0,
            last_attempt_at: None,
            payment_id: None,
            failure_reason: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        self.scheduled_payment_repo.create(scheduled).await
    }

    /// Migrate a Stripe-subscription member to Coterie-managed
    /// auto-renew. The whole flow:
    ///
    /// 1. Pull every card on file from Stripe (we don't want to make
    ///    the member re-enter card data) and hydrate Coterie's
    ///    SavedCard table for any new ones.
    /// 2. Cancel the Stripe subscription so Stripe stops charging.
    /// 3. Atomically flip billing_mode → coterie_managed and clear
    ///    stripe_subscription_id. The webhook handler is hardened
    ///    to skip already-migrated members so it won't undo this.
    /// 4. Schedule a fresh renewal against the member's existing
    ///    dues_paid_until — they keep their paid-through date and
    ///    Coterie takes over the next charge.
    ///
    /// No-op if the member isn't on stripe_subscription, or doesn't
    /// have a stripe_customer_id / stripe_subscription_id we need.
    /// Returns Ok(false) in those cases so callers can chain it
    /// safely (e.g. the save_card handler runs this for everyone but
    /// only stripe-sub members actually migrate).
    pub async fn migrate_to_coterie_managed(
        &self,
        member_id: Uuid,
    ) -> Result<bool> {
        let member = self.member_repo.find_by_id(member_id).await?
            .ok_or_else(|| AppError::NotFound("Member not found".to_string()))?;

        if member.billing_mode != BillingMode::StripeSubscription {
            return Ok(false);
        }

        let customer_id = member.stripe_customer_id
            .as_deref()
            .ok_or_else(|| AppError::Internal(format!(
                "Member {} is on stripe_subscription but has no stripe_customer_id", member_id
            )))?;
        let subscription_id = member.stripe_subscription_id
            .as_deref()
            .ok_or_else(|| AppError::Internal(format!(
                "Member {} is on stripe_subscription but has no stripe_subscription_id", member_id
            )))?;

        let stripe = self.stripe_client.as_ref().ok_or_else(|| {
            AppError::ServiceUnavailable("Stripe not configured".to_string())
        })?;

        // 1. Hydrate SavedCards from Stripe — do this BEFORE
        // cancelling so a Stripe outage doesn't leave the member
        // with a cancelled sub and no cards in Coterie.
        let stripe_cards = stripe.list_customer_cards(customer_id).await?;
        let existing = self.saved_card_repo.find_by_member(member_id).await?;
        let now = Utc::now();

        for card in &stripe_cards {
            let already_have = existing.iter()
                .any(|c| c.stripe_payment_method_id == card.payment_method_id);
            if already_have {
                continue;
            }
            // First insert with is_default=false; we'll fix the
            // default flag in one place below to avoid duplicates.
            let saved = SavedCard {
                id: Uuid::new_v4(),
                member_id,
                stripe_payment_method_id: card.payment_method_id.clone(),
                card_last_four: card.last_four.clone(),
                card_brand: card.brand.clone(),
                exp_month: card.exp_month,
                exp_year: card.exp_year,
                is_default: false,
                created_at: now,
                updated_at: now,
            };
            self.saved_card_repo.create(saved).await?;
        }

        // Pick the default card. Prefer the one Stripe says is the
        // customer's default; fall back to whatever's already marked
        // default in Coterie; fall back to the most recently added.
        let all_cards = self.saved_card_repo.find_by_member(member_id).await?;
        let stripe_default_pm_id = stripe_cards.iter()
            .find(|c| c.is_default)
            .map(|c| c.payment_method_id.clone());

        let default_card_id: Option<Uuid> = stripe_default_pm_id
            .and_then(|pm| {
                all_cards.iter()
                    .find(|c| c.stripe_payment_method_id == pm)
                    .map(|c| c.id)
            })
            .or_else(|| all_cards.iter().find(|c| c.is_default).map(|c| c.id))
            .or_else(|| all_cards.last().map(|c| c.id));

        if let Some(card_id) = default_card_id {
            // set_default both flips the chosen card to default=true
            // and clears it on every other card for this member.
            self.saved_card_repo.set_default(member_id, card_id).await?;
        }

        // 2. Flip local state FIRST, before Stripe cancellation.
        // Stripe fires customer.subscription.deleted essentially
        // simultaneously with the cancel_subscription return — if we
        // cancel before flipping, the webhook can land between the
        // two calls, read billing_mode='stripe_subscription' (still!),
        // interpret as out-of-band cancellation, and email the member
        // a misleading "auto-renew cancelled" message. By flipping
        // first, the webhook reads coterie_managed and skips its
        // notify path.
        let stashed_sub_id = subscription_id.to_string();
        self.member_repo
            .set_billing_mode(member_id, BillingMode::CoterieManaged, None)
            .await?;

        // 3. Cancel the Stripe subscription. If this fails, roll
        // back the local flip so the operator can retry — leaving
        // local in coterie_managed while Stripe still bills would
        // be the worst of both worlds.
        if let Err(e) = stripe.cancel_subscription(&stashed_sub_id).await {
            self.member_repo
                .set_billing_mode(member_id, BillingMode::StripeSubscription, Some(&stashed_sub_id))
                .await
                .map_err(|rollback| AppError::Internal(format!(
                    "Stripe cancel failed ({}) AND local rollback failed ({}); \
                     member {} is in coterie_managed but Stripe is still \
                     billing them. Manual intervention required.",
                    e, rollback, member_id,
                )))?;
            return Err(e);
        }

        // 4. Schedule the next renewal against current dues_paid_until.
        // Cancel any stale scheduled_payments first (defensive — there
        // shouldn't be any for a stripe_sub member, but better safe).
        self.cancel_scheduled_payments(member_id).await?;

        let mt_id = member.membership_type_id
            .ok_or_else(|| AppError::Internal(
                "Member has no membership_type_id; cannot schedule renewal".to_string()
            ))?;
        let mt = self.membership_type_service.get(mt_id).await?
            .ok_or_else(|| AppError::Internal(
                "Member's membership type was deleted".to_string()
            ))?;
        self.schedule_renewal(member_id, &mt.slug).await?;

        tracing::info!(
            "Migrated member {} from stripe_subscription to coterie_managed \
             (cards imported: {})",
            member_id, stripe_cards.len(),
        );

        Ok(true)
    }

    /// Migrate every member currently on `stripe_subscription` to
    /// `coterie_managed`. Per-member failures don't stop the run —
    /// each member's outcome is logged + collected in the summary so
    /// the admin can spot stragglers.
    ///
    /// Returns (succeeded, skipped, failed) where:
    ///   - succeeded: migration ran and returned Ok(true)
    ///   - skipped: migration ran and returned Ok(false) (member
    ///     wasn't actually on stripe_sub by the time we got there)
    ///   - failed: a list of (member_id, error) for things that blew up
    pub async fn bulk_migrate_stripe_subscriptions(&self) -> BulkMigrationSummary {
        let mut summary = BulkMigrationSummary::default();

        let ids = match self.member_repo
            .list_ids_by_billing_mode(BillingMode::StripeSubscription)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("bulk_migrate_stripe_subscriptions: list query failed: {}", e);
                summary.failed.push((Uuid::nil(), format!("DB error: {}", e)));
                return summary;
            }
        };

        for member_id in ids {
            match self.migrate_to_coterie_managed(member_id).await {
                Ok(true) => summary.succeeded += 1,
                Ok(false) => summary.skipped += 1,
                Err(e) => {
                    tracing::error!(
                        "Bulk migrate: member {} failed: {}",
                        member_id, e,
                    );
                    summary.failed.push((member_id, e.to_string()));
                }
            }
        }
        summary
    }

    /// Whether the member is currently enrolled in Coterie-managed
    /// auto-renewal. We treat StripeSubscription as a separate path
    /// (charges happen on Stripe's side, no ScheduledPayment row), so
    /// this returns true only for `coterie_managed`.
    pub async fn is_auto_renew(&self, member_id: Uuid) -> Result<bool> {
        let mode = self.member_repo.find_by_id(member_id).await?
            .map(|m| m.billing_mode);
        Ok(mode == Some(BillingMode::CoterieManaged))
    }

    /// Transition a member onto Coterie-managed auto-renewal.
    /// Idempotent — if they're already enrolled, we still cancel any
    /// stale pending scheduled payments and queue a fresh one based on
    /// the member's current `dues_paid_until`. Cancel-first guarantees
    /// we never leave two pending charges for the same cycle (e.g.,
    /// from a double-submit).
    ///
    /// For members currently on `stripe_subscription`, this delegates
    /// to `migrate_to_coterie_managed` instead of just flipping the
    /// flag — otherwise we'd double-bill them (Stripe's existing
    /// subscription would still charge alongside our schedule).
    pub async fn enable_auto_renew(
        &self,
        member_id: Uuid,
        membership_type_slug: &str,
    ) -> Result<()> {
        let member = self.member_repo.find_by_id(member_id).await?
            .ok_or_else(|| AppError::NotFound("Member not found".to_string()))?;

        if member.billing_mode == BillingMode::StripeSubscription {
            self.migrate_to_coterie_managed(member_id).await?;
            return Ok(());
        }

        self.cancel_scheduled_payments(member_id).await?;

        // Preserve any existing stripe_subscription_id — leaving it
        // intact is harmless since the mode discriminator gates the
        // billing pathways. We only clear it on the migrate or
        // disable paths, which actually cancel the Stripe sub.
        let prior_sub_id = member.stripe_subscription_id.clone();
        self.member_repo
            .set_billing_mode(member_id, BillingMode::CoterieManaged, prior_sub_id.as_deref())
            .await?;

        self.schedule_renewal(member_id, membership_type_slug).await?;
        Ok(())
    }

    /// Replace an enrolled member's pending scheduled payment with a
    /// fresh one based on their current `dues_paid_until`. Used when
    /// the member pays *early* (e.g., via one-time Checkout) — the
    /// previously queued ScheduledPayment now points at the wrong due
    /// date and would over-charge them.
    ///
    /// No-op if the member isn't on coterie_managed: we don't queue
    /// payments for manual or stripe_subscription members.
    pub async fn reschedule_after_payment(
        &self,
        member_id: Uuid,
        membership_type_slug: &str,
    ) -> Result<()> {
        if !self.is_auto_renew(member_id).await? {
            return Ok(());
        }
        self.cancel_scheduled_payments(member_id).await?;
        self.schedule_renewal(member_id, membership_type_slug).await?;
        Ok(())
    }

    /// Move the member off any form of auto-renewal — handles both
    /// coterie_managed (cancel pending scheduled_payments) AND
    /// stripe_subscription (cancel the Stripe sub via API). Either
    /// way they end up at billing_mode='manual'. Saved cards stay
    /// on file for future one-off payments.
    ///
    /// "Off" should really mean off — for a stripe-sub member, just
    /// flipping local state without cancelling Stripe would let
    /// Stripe keep charging them, which is the opposite of what
    /// "disable auto-renew" means.
    pub async fn disable_auto_renew(&self, member_id: Uuid) -> Result<()> {
        let member = self.member_repo.find_by_id(member_id).await?
            .ok_or_else(|| AppError::NotFound("Member not found".to_string()))?;

        // Cancel Coterie-side pending charges no matter what — cheap
        // and safe to call when there are none.
        self.cancel_scheduled_payments(member_id).await?;

        // Flip local state FIRST so the customer.subscription.deleted
        // webhook (which Stripe fires near-simultaneously with our
        // cancel_subscription return) reads billing_mode='manual' and
        // skips the "out-of-band cancellation" notify path. If we
        // cancelled before flipping, the webhook would email the
        // member a contradictory "your auto-renew was cancelled"
        // message during a flow they explicitly initiated.
        let stripe_sub_to_cancel: Option<String> =
            if member.billing_mode == BillingMode::StripeSubscription {
                member.stripe_subscription_id.clone()
            } else {
                None
            };
        let prior_mode = member.billing_mode;
        let prior_sub_id = member.stripe_subscription_id.clone();

        self.member_repo
            .set_billing_mode(member_id, BillingMode::Manual, None)
            .await?;

        // Cancel the Stripe subscription if the member had one. On
        // failure, roll back so the operator can retry without us
        // leaving them in 'manual' while Stripe keeps billing.
        if let Some(sub_id) = stripe_sub_to_cancel {
            let stripe = self.stripe_client.as_ref().ok_or_else(|| {
                AppError::ServiceUnavailable("Stripe not configured".to_string())
            })?;
            if let Err(e) = stripe.cancel_subscription(&sub_id).await {
                self.member_repo
                    .set_billing_mode(member_id, prior_mode, prior_sub_id.as_deref())
                    .await
                    .map_err(|rollback| AppError::Internal(format!(
                        "Stripe cancel failed ({}) AND local rollback failed ({}); \
                         member {} is in 'manual' but Stripe may still bill them. \
                         Manual intervention required.",
                        e, rollback, member_id,
                    )))?;
                return Err(e);
            }
        }

        Ok(())
    }

    /// Cancel all pending scheduled payments for a member.
    pub async fn cancel_scheduled_payments(&self, member_id: Uuid) -> Result<u32> {
        let pending = self
            .scheduled_payment_repo
            .find_by_member(member_id)
            .await?;
        let mut count = 0;
        for sp in pending {
            if sp.status == ScheduledPaymentStatus::Pending {
                self.scheduled_payment_repo
                    .update_status(sp.id, ScheduledPaymentStatus::Canceled, None)
                    .await?;
                count += 1;
            }
        }
        Ok(count)
    }

    /// Process a single scheduled payment: charge the member's default card.
    pub async fn process_scheduled_payment(&self, id: Uuid) -> Result<()> {
        let sp = self
            .scheduled_payment_repo
            .find_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound("Scheduled payment not found".to_string()))?;

        if sp.status != ScheduledPaymentStatus::Pending {
            return Err(AppError::BadRequest(format!(
                "Scheduled payment is {:?}, not pending",
                sp.status
            )));
        }

        let stripe_client = self.stripe_client.as_ref().ok_or_else(|| {
            AppError::ServiceUnavailable("Stripe not configured".to_string())
        })?;

        // Mark as processing
        self.scheduled_payment_repo
            .update_status(id, ScheduledPaymentStatus::Processing, None)
            .await?;

        // Find the member's default card
        let default_card = self
            .saved_card_repo
            .find_default_for_member(sp.member_id)
            .await?;

        let card = match default_card {
            Some(c) => c,
            None => {
                self.scheduled_payment_repo
                    .update_status(
                        id,
                        ScheduledPaymentStatus::Failed,
                        Some("No default payment method".to_string()),
                    )
                    .await?;
                return Ok(());
            }
        };

        // Don't waste a Stripe round-trip (or a declined-card footprint
        // on the member's bank) on a card we can already see is expired.
        // The reminder job has a parallel check that nags these members
        // ahead of time so in theory it shouldn't come to this.
        if !card.is_valid_at(Utc::now()) {
            self.scheduled_payment_repo
                .update_status(
                    id,
                    ScheduledPaymentStatus::Failed,
                    Some(format!(
                        "Default card expired ({} {})",
                        card.display_name(),
                        card.exp_display(),
                    )),
                )
                .await?;
            return Ok(());
        }

        // Look up membership type name for description
        let membership_type = self
            .membership_type_service
            .get(sp.membership_type_id)
            .await?;
        let description = format!(
            "{} membership renewal",
            membership_type
                .as_ref()
                .map(|mt| mt.name.as_str())
                .unwrap_or("Membership")
        );

        // Use the scheduled-payment ID as the idempotency key — if the billing
        // runner fires twice for the same scheduled payment, Stripe dedupes.
        let idempotency_key = format!("sched-{}", sp.id);

        // Pre-allocate the Payment row ID. We pass it into Stripe as
        // metadata.payment_id so the payment_intent.succeeded webhook
        // can correlate; we use the same ID below when persisting the
        // row. The runner's retry-with-same-idempotency-key gives us
        // recovery if the row insert fails after the charge succeeds.
        let payment_id = Uuid::new_v4();

        // Attempt the charge
        match stripe_client
            .charge_saved_card(
                sp.member_id,
                &card.stripe_payment_method_id,
                sp.amount_cents,
                &description,
                &idempotency_key,
                payment_id,
            )
            .await
        {
            Ok(stripe_payment_id) => {
                // Create payment record
                let payment = Payment {
                    id: payment_id,
                    member_id: Some(sp.member_id),
                    amount_cents: sp.amount_cents,
                    currency: sp.currency.clone(),
                    status: PaymentStatus::Completed,
                    payment_method: PaymentMethod::Stripe,
                    stripe_payment_id: Some(stripe_payment_id),
                    description,
                    payment_type: PaymentType::Membership,
                    donation_campaign_id: None,
                    donor_name: None,
                    donor_email: None,
                    paid_at: Some(Utc::now()),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                };

                let payment = self.payment_repo.create(payment).await?;

                // Link payment and mark completed
                self.scheduled_payment_repo
                    .link_payment(id, payment.id)
                    .await?;
                self.scheduled_payment_repo
                    .update_status(id, ScheduledPaymentStatus::Completed, None)
                    .await?;

                // Extend dues
                self.extend_member_dues(payment.id, sp.member_id, sp.membership_type_id)
                    .await?;

                // Schedule next renewal. Failure here doesn't roll back
                // the successful charge, but the member would fall off
                // the auto-renew cycle silently. Dispatch an AdminAlert
                // so an operator notices and can re-queue them — the
                // log alone isn't enough because nobody reads logs
                // until a member complains months later.
                if let Some(mt) = &membership_type {
                    if let Err(e) = self.schedule_renewal(sp.member_id, &mt.slug).await {
                        tracing::error!(
                            "Charged member {} (sp {}) but failed to schedule next renewal: {}",
                            sp.member_id, id, e
                        );
                        let member_label = self.member_repo
                            .find_by_id(sp.member_id).await
                            .ok().flatten()
                            .map(|m| format!("{} <{}>", m.full_name, m.email))
                            .unwrap_or_else(|| sp.member_id.to_string());
                        self.integration_manager
                            .handle_event(IntegrationEvent::AdminAlert {
                                subject: format!(
                                    "Auto-renew schedule failed after charge — {}",
                                    member_label,
                                ),
                                body: format!(
                                    "Member: {}\n\
                                     Charged scheduled payment {} successfully, \
                                     but failed to queue the NEXT renewal: {}\n\
                                     \n\
                                     The member's dues are paid through the new \
                                     period, but they're now off the auto-renew \
                                     loop. Re-enroll them via /portal/admin/members/{}/.",
                                    member_label, id, e, sp.member_id,
                                ),
                            })
                            .await;
                    }
                }

                tracing::info!(
                    "Processed scheduled payment {} for member {}",
                    id,
                    sp.member_id
                );
            }
            Err(e) => {
                let max_retries = self.get_max_retries().await;
                self.scheduled_payment_repo.increment_retry(id).await?;

                if sp.retry_count + 1 >= max_retries {
                    self.scheduled_payment_repo
                        .update_status(
                            id,
                            ScheduledPaymentStatus::Failed,
                            Some(format!("Max retries exceeded: {}", e)),
                        )
                        .await?;
                    tracing::warn!(
                        "Scheduled payment {} failed permanently for member {}: {}",
                        id,
                        sp.member_id,
                        e
                    );
                } else {
                    // Back to pending for retry
                    self.scheduled_payment_repo
                        .update_status(
                            id,
                            ScheduledPaymentStatus::Pending,
                            Some(format!("{}", e)),
                        )
                        .await?;
                    tracing::warn!(
                        "Scheduled payment {} failed (retry {}/{}): {}",
                        id,
                        sp.retry_count + 1,
                        max_retries,
                        e
                    );
                }
            }
        }

        Ok(())
    }

    /// Run the billing cycle: find all due payments and process them.
    pub async fn run_billing_cycle(&self) -> Result<(u32, u32)> {
        let today = Utc::now().date_naive();
        let pending = self
            .scheduled_payment_repo
            .find_pending_due_before(today)
            .await?;

        let total = pending.len() as u32;
        let mut succeeded = 0u32;

        for sp in pending {
            match self.process_scheduled_payment(sp.id).await {
                Ok(()) => {
                    // Check if it completed (vs failed-but-handled)
                    if let Ok(Some(updated)) =
                        self.scheduled_payment_repo.find_by_id(sp.id).await
                    {
                        if updated.status == ScheduledPaymentStatus::Completed {
                            succeeded += 1;
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(
                        "Error processing scheduled payment {}: {}",
                        sp.id,
                        e
                    );
                }
            }
        }

        tracing::info!(
            "Billing cycle complete: {}/{} succeeded",
            succeeded,
            total
        );
        Ok((succeeded, total))
    }

    /// Slug-based variant of `extend_member_dues`. Convenient for
    /// callers that already have the membership type slug (admin
    /// manual-payment, waive, Stripe checkout success) and don't want
    /// to do a separate slug→id round-trip first.
    pub async fn extend_member_dues_by_slug(
        &self,
        payment_id: Uuid,
        member_id: Uuid,
        membership_type_slug: &str,
    ) -> Result<()> {
        let mt = self.membership_type_service
            .get_by_slug(membership_type_slug)
            .await?
            .ok_or_else(|| AppError::NotFound(format!(
                "Membership type '{}' not found", membership_type_slug
            )))?;
        self.extend_member_dues(payment_id, member_id, mt.id).await
    }

    pub async fn extend_member_dues(
        &self,
        payment_id: Uuid,
        member_id: Uuid,
        membership_type_id: Uuid,
    ) -> Result<()> {
        let membership_type = self
            .membership_type_service
            .get(membership_type_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Membership type not found".to_string()))?;

        let billing_period = membership_type
            .billing_period_enum()
            .unwrap_or(BillingPeriod::Yearly);

        // Atomic per-payment claim + member update — see
        // PaymentRepository::extend_dues_for_payment_atomic for why
        // this isn't a SELECT/compute/UPDATE pair anymore.
        let extended = self.payment_repo
            .extend_dues_for_payment_atomic(payment_id, member_id, billing_period)
            .await?;

        if extended {
            tracing::info!(
                "Extended dues for member {} (payment: {}, billing period: {:?})",
                member_id, payment_id, billing_period,
            );
        } else {
            tracing::debug!(
                "Dues already extended for payment {}; skipping",
                payment_id,
            );
        }

        Ok(())
    }

    async fn get_max_retries(&self) -> i32 {
        self.settings_service
            .get_number("billing.max_retry_attempts")
            .await
            .unwrap_or(3) as i32
    }
}
