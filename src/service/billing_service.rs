use chrono::{Months, NaiveDate, Utc};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    domain::{
        configurable_types::BillingPeriod, BillingMode, Payment, PaymentMethod, PaymentStatus,
        PaymentType, SavedCard, ScheduledPayment, ScheduledPaymentStatus,
    },
    email::EmailSender,
    error::{AppError, Result},
    integrations::{IntegrationEvent, IntegrationManager},
    payments::StripeClient,
    repository::{MemberRepository, PaymentRepository, SavedCardRepository, ScheduledPaymentRepository},
    service::{membership_type_service::MembershipTypeService, settings_service::SettingsService},
};
use sqlx::SqlitePool;

/// Result of `bulk_migrate_stripe_subscriptions` — per-batch summary
/// the admin UI displays after running the migration.
#[derive(Debug, Default)]
pub struct BulkMigrationSummary {
    pub succeeded: u32,
    pub skipped: u32,
    pub failed: Vec<(Uuid, String)>,
}

pub struct BillingService {
    scheduled_payment_repo: Arc<dyn ScheduledPaymentRepository>,
    payment_repo: Arc<dyn PaymentRepository>,
    saved_card_repo: Arc<dyn SavedCardRepository>,
    member_repo: Arc<dyn MemberRepository>,
    membership_type_service: Arc<MembershipTypeService>,
    settings_service: Arc<SettingsService>,
    email_sender: Arc<dyn EmailSender>,
    integration_manager: Arc<IntegrationManager>,
    stripe_client: Option<Arc<StripeClient>>,
    /// Absolute URL to this Coterie instance — used to build links in
    /// outgoing reminder emails. Comes from ServerConfig::base_url.
    base_url: String,
    db_pool: SqlitePool,
}

impl BillingService {
    pub fn new(
        scheduled_payment_repo: Arc<dyn ScheduledPaymentRepository>,
        payment_repo: Arc<dyn PaymentRepository>,
        saved_card_repo: Arc<dyn SavedCardRepository>,
        member_repo: Arc<dyn MemberRepository>,
        membership_type_service: Arc<MembershipTypeService>,
        settings_service: Arc<SettingsService>,
        email_sender: Arc<dyn EmailSender>,
        integration_manager: Arc<IntegrationManager>,
        stripe_client: Option<Arc<StripeClient>>,
        base_url: String,
        db_pool: SqlitePool,
    ) -> Self {
        Self {
            scheduled_payment_repo,
            payment_repo,
            saved_card_repo,
            member_repo,
            membership_type_service,
            settings_service,
            email_sender,
            integration_manager,
            stripe_client,
            base_url,
            db_pool,
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

        // Get current dues_paid_until to determine next due date
        let dues_paid_until: Option<String> = sqlx::query_scalar(
            "SELECT dues_paid_until FROM members WHERE id = ?",
        )
        .bind(member_id.to_string())
        .fetch_optional(&self.db_pool)
        .await
        .map_err(|e| AppError::Internal(format!("Database error: {}", e)))?
        .flatten();

        let next_due = if let Some(due_str) = dues_paid_until {
            NaiveDate::parse_from_str(&due_str[..10], "%Y-%m-%d")
                .unwrap_or_else(|_| Utc::now().date_naive())
        } else {
            Utc::now().date_naive()
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

        // 2. Cancel the Stripe subscription. If this fails, bail
        // before touching local state — the member stays on
        // stripe_subscription and the operator can retry.
        stripe.cancel_subscription(subscription_id).await?;

        // 3. Flip local state. The CASE guard makes this idempotent
        // against the inevitable customer.subscription.deleted
        // webhook: if it fires AFTER this update lands, the
        // hardened webhook handler will see coterie_managed and
        // leave us alone.
        sqlx::query(
            "UPDATE members \
             SET billing_mode = 'coterie_managed', \
                 stripe_subscription_id = NULL, \
                 updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?",
        )
        .bind(member_id.to_string())
        .execute(&self.db_pool)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to flip billing_mode: {}", e)))?;

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

        let rows: Vec<(String,)> = match sqlx::query_as(
            "SELECT id FROM members WHERE billing_mode = 'stripe_subscription'",
        )
        .fetch_all(&self.db_pool)
        .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("bulk_migrate_stripe_subscriptions: list query failed: {}", e);
                summary.failed.push((Uuid::nil(), format!("DB error: {}", e)));
                return summary;
            }
        };

        for (id_str,) in rows {
            let member_id = match Uuid::parse_str(&id_str) {
                Ok(id) => id,
                Err(_) => continue,
            };
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

    /// The member cancelled their Stripe subscription out-of-band
    /// (e.g., via Stripe's customer portal). Emails them so they
    /// know auto-renewal is off but they still have access through
    /// their paid-through date, plus dispatches an AdminAlert so
    /// operators see the churn signal.
    ///
    /// Caller is responsible for the local-state flip
    /// (billing_mode='manual', clear sub_id) — this method is just
    /// notification.
    pub async fn notify_subscription_cancelled(&self, member_id: Uuid) -> Result<()> {
        use crate::email::{self, templates::{SubscriptionCancelledHtml, SubscriptionCancelledText}};

        let member = self.member_repo.find_by_id(member_id).await?
            .ok_or_else(|| AppError::NotFound("Member not found".to_string()))?;

        let org_name = self.settings_service
            .get_value("org.name").await
            .ok().filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Coterie".to_string());

        let portal_url = format!(
            "{}/portal/payments/methods",
            self.base_url.trim_end_matches('/'),
        );

        let dues_until = member.dues_paid_until
            .map(|d| d.format("%B %d, %Y").to_string())
            .unwrap_or_else(|| "(unknown)".to_string());

        let html = SubscriptionCancelledHtml {
            full_name: &member.full_name,
            org_name: &org_name,
            dues_until: &dues_until,
            portal_url: &portal_url,
        };
        let text = SubscriptionCancelledText {
            full_name: &member.full_name,
            org_name: &org_name,
            dues_until: &dues_until,
            portal_url: &portal_url,
        };
        let subject = format!("Your {} auto-renewal has been turned off", org_name);

        let message = email::message_from_templates(
            member.email.clone(), subject, &html, &text,
        )?;
        if let Err(e) = self.email_sender.send(&message).await {
            tracing::error!(
                "Couldn't email subscription-cancelled notice to member {}: {}",
                member_id, e,
            );
        }

        let alert_body = format!(
            "Member: {} <{}>\n\
             Their Stripe subscription was cancelled out-of-band. They've\n\
             been emailed and switched to manual billing. Access continues\n\
             through {}.",
            member.full_name, member.email, dues_until,
        );
        self.integration_manager
            .handle_event(IntegrationEvent::AdminAlert {
                subject: format!("Stripe subscription cancelled — {}", member.full_name),
                body: alert_body,
            })
            .await;

        Ok(())
    }

    /// A Stripe-managed subscription charge failed. Emails the member
    /// directly so they can update their card, and dispatches an
    /// AdminAlert so operators get a heads-up via Discord/email.
    ///
    /// `is_final` should be true when Stripe has exhausted retries —
    /// we soften the email copy in that case ("this was the last
    /// attempt"). Doesn't touch dues_paid_until or member status:
    /// the natural expiration job will handle them once the paid-
    /// through date lapses.
    pub async fn notify_subscription_payment_failed(
        &self,
        member_id: Uuid,
        amount_display: Option<String>,
        is_final: bool,
    ) -> Result<()> {
        use crate::email::{self, templates::{CardDeclinedHtml, CardDeclinedText}};

        let member = self.member_repo.find_by_id(member_id).await?
            .ok_or_else(|| AppError::NotFound("Member not found".to_string()))?;

        let org_name = self.settings_service
            .get_value("org.name").await
            .ok().filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Coterie".to_string());

        let base = self.base_url.trim_end_matches('/');
        let portal_url = format!("{}/portal/payments/methods", base);

        let dues_until = member.dues_paid_until
            .map(|d| d.format("%B %d, %Y").to_string())
            .unwrap_or_else(|| "(unknown)".to_string());

        let html = CardDeclinedHtml {
            full_name: &member.full_name,
            org_name: &org_name,
            amount: amount_display.as_deref(),
            portal_url: &portal_url,
            dues_until: &dues_until,
            is_final,
        };
        let text = CardDeclinedText {
            full_name: &member.full_name,
            org_name: &org_name,
            amount: amount_display.as_deref(),
            portal_url: &portal_url,
            dues_until: &dues_until,
            is_final,
        };

        let subject = if is_final {
            format!("Final notice: card declined for {} membership", org_name)
        } else {
            format!("Card declined for {} membership", org_name)
        };

        let message = email::message_from_templates(
            member.email.clone(), subject, &html, &text,
        )?;

        if let Err(e) = self.email_sender.send(&message).await {
            tracing::error!(
                "Couldn't email card-declined notice to member {}: {}",
                member_id, e,
            );
        }

        // AdminAlert: the body is plain-text since it's piped into
        // both Discord (markdown-ish) and email.
        let alert_subject = if is_final {
            format!("Stripe subscription charge failed (final) — {}", member.full_name)
        } else {
            format!("Stripe subscription charge failed — {}", member.full_name)
        };
        let alert_body = format!(
            "Member: {} <{}>\n\
             Amount: {}\n\
             Status: {}\n\
             Member has been emailed at their address on file.",
            member.full_name,
            member.email,
            amount_display.as_deref().unwrap_or("(unknown)"),
            if is_final {
                "Stripe exhausted retries — member must manually re-pay or the membership will lapse"
            } else {
                "Stripe will retry automatically; member can update card to fix"
            },
        );
        self.integration_manager
            .handle_event(IntegrationEvent::AdminAlert {
                subject: alert_subject,
                body: alert_body,
            })
            .await;

        Ok(())
    }

    /// Whether the member is currently enrolled in Coterie-managed
    /// auto-renewal. We treat StripeSubscription as a separate path
    /// (charges happen on Stripe's side, no ScheduledPayment row), so
    /// this returns true only for `coterie_managed`.
    pub async fn is_auto_renew(&self, member_id: Uuid) -> Result<bool> {
        let mode: Option<String> = sqlx::query_scalar(
            "SELECT billing_mode FROM members WHERE id = ?",
        )
        .bind(member_id.to_string())
        .fetch_optional(&self.db_pool)
        .await
        .map_err(|e| AppError::Internal(format!("Database error: {}", e)))?;

        Ok(mode.as_deref() == Some("coterie_managed"))
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

        sqlx::query(
            "UPDATE members \
             SET billing_mode = 'coterie_managed', updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?",
        )
        .bind(member_id.to_string())
        .execute(&self.db_pool)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to set billing_mode: {}", e)))?;

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

        // Cancel any active Stripe subscription. If Stripe rejects
        // the cancel (e.g., the sub_id stale), bail BEFORE touching
        // local state so the operator can investigate without us
        // leaving a half-migrated member.
        if member.billing_mode == BillingMode::StripeSubscription {
            if let Some(sub_id) = member.stripe_subscription_id.as_deref() {
                let stripe = self.stripe_client.as_ref().ok_or_else(|| {
                    AppError::ServiceUnavailable("Stripe not configured".to_string())
                })?;
                stripe.cancel_subscription(sub_id).await?;
            }
        }

        sqlx::query(
            "UPDATE members \
             SET billing_mode = 'manual', \
                 stripe_subscription_id = NULL, \
                 updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?",
        )
        .bind(member_id.to_string())
        .execute(&self.db_pool)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to clear billing_mode: {}", e)))?;
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

        // Attempt the charge
        match stripe_client
            .charge_saved_card(
                sp.member_id,
                &card.stripe_payment_method_id,
                sp.amount_cents,
                &description,
                &idempotency_key,
            )
            .await
        {
            Ok(stripe_payment_id) => {
                // Create payment record
                let payment = Payment {
                    id: Uuid::new_v4(),
                    member_id: sp.member_id,
                    amount_cents: sp.amount_cents,
                    currency: sp.currency.clone(),
                    status: PaymentStatus::Completed,
                    payment_method: PaymentMethod::Stripe,
                    stripe_payment_id: Some(stripe_payment_id),
                    description,
                    payment_type: PaymentType::Membership,
                    donation_campaign_id: None,
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
                self.extend_member_dues(sp.member_id, sp.membership_type_id)
                    .await?;

                // Schedule next renewal. Failure here doesn't roll back
                // the successful charge, but it does mean the member
                // falls off the auto-renew cycle silently — log so an
                // operator can re-queue them by hand.
                if let Some(mt) = &membership_type {
                    if let Err(e) = self.schedule_renewal(sp.member_id, &mt.slug).await {
                        tracing::error!(
                            "Charged member {} (sp {}) but failed to schedule next renewal: {}",
                            sp.member_id, id, e
                        );
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

    /// Check for members past grace period and expire them. Also kills
    /// any live sessions for the affected members so they stop having
    /// portal access on the next request rather than the one after.
    pub async fn check_expired_members(&self) -> Result<u32> {
        let grace_days = self.get_grace_period_days().await;

        // UPDATE...RETURNING gives us the affected IDs in one round-trip
        // so we can invalidate their sessions below.
        let expired_ids: Vec<(String,)> = sqlx::query_as(
            r#"
            UPDATE members
            SET status = 'Expired', updated_at = CURRENT_TIMESTAMP
            WHERE status = 'Active'
              AND dues_paid_until IS NOT NULL
              AND date(dues_paid_until, '+' || ? || ' days') < date('now')
              AND bypass_dues = 0
            RETURNING id
            "#,
        )
        .bind(grace_days)
        .fetch_all(&self.db_pool)
        .await
        .map_err(|e| AppError::Internal(format!("Database error: {}", e)))?;

        let expired_count = expired_ids.len() as u32;

        // Force-logout expired members. `require_auth_redirect` would
        // bounce them to /portal/restore on their next request anyway,
        // but killing the session makes the expiration immediate from
        // the browser's perspective.
        //
        // Placeholder count is derived from our own DB row count, not
        // user input — format!() is safe here.
        if !expired_ids.is_empty() {
            let placeholders = vec!["?"; expired_ids.len()].join(",");
            let sql = format!("DELETE FROM sessions WHERE member_id IN ({})", placeholders);
            let mut q = sqlx::query(&sql);
            for (id,) in &expired_ids {
                q = q.bind(id);
            }
            if let Err(e) = q.execute(&self.db_pool).await {
                tracing::warn!(
                    "Marked {} members Expired but session cleanup failed: {}. \
                     Middleware still rejects Expired status, so members are \
                     bounced to /portal/restore on next request.",
                    expired_count, e
                );
            }
        }

        // Fire MemberExpired events so integrations (Discord role swap,
        // future Unifi access revocation) can react. Best-effort — a
        // failure here doesn't roll back the expiration.
        for (id_str,) in &expired_ids {
            if let Ok(uuid) = Uuid::parse_str(id_str) {
                if let Ok(Some(member)) = self.member_repo.find_by_id(uuid).await {
                    self.integration_manager
                        .handle_event(IntegrationEvent::MemberExpired(member))
                        .await;
                }
            }
        }

        if expired_count > 0 {
            tracing::info!(
                "Expired {} members past grace period ({} days); sessions invalidated",
                expired_count,
                grace_days
            );
        }

        Ok(expired_count)
    }

    /// Find members whose dues will lapse within the reminder window
    /// and email each one — or skip them silently if they're on
    /// auto-renew with a valid card for a short-period plan.
    ///
    /// Four cases:
    ///  1. Manual billing → "pay your dues" reminder.
    ///  2. Auto-renew, card will be valid at charge time, period
    ///     <= monthly → skip (the charge will just happen).
    ///  3. Auto-renew, valid card, period >= yearly → "heads up,
    ///     we're going to auto-charge you $X" notice.
    ///  4. Auto-renew, card expired/missing by charge time → same
    ///     reminder as case 1 but with a "your card is invalid"
    ///     callout so the member knows auto-charge won't save them.
    ///
    /// Idempotent per cycle via `dues_reminder_sent_at`. Case 2 does
    /// NOT set the flag — we want those members to become eligible
    /// again if their card or billing mode changes mid-window.
    pub async fn send_dues_reminders(&self) -> Result<u32> {
        use crate::{
            domain::{configurable_types::BillingPeriod, BillingMode},
            email::{self, templates::{ReminderHtml, ReminderText, RenewalNoticeHtml, RenewalNoticeText}},
        };

        let reminder_days = self.settings_service
            .get_number("membership.reminder_days_before")
            .await
            .unwrap_or(7)
            .clamp(1, 90);

        let org_name = self.settings_service
            .get_value("org.name").await
            .ok().filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Coterie".to_string());

        let base = self.base_url.trim_end_matches('/');
        let pay_url = format!("{}/portal/payments/new", base);
        let portal_url = format!("{}/portal/payments/methods", base);

        // Candidate members: Active, dues in the window, not yet reminded.
        // We fetch billing_mode and membership_type_id here so we can
        // branch in code rather than doing N+1 joins.
        let rows: Vec<(String, String, String, chrono::NaiveDateTime, String, Option<String>)> =
            sqlx::query_as(
                r#"
                SELECT id, email, full_name, dues_paid_until, billing_mode, membership_type_id
                FROM members
                WHERE status = 'Active'
                  AND bypass_dues = 0
                  AND dues_paid_until IS NOT NULL
                  AND dues_paid_until > CURRENT_TIMESTAMP
                  AND date(dues_paid_until) <= date(CURRENT_TIMESTAMP, '+' || ? || ' days')
                  AND dues_reminder_sent_at IS NULL
                "#
            )
            .bind(reminder_days)
            .fetch_all(&self.db_pool)
            .await
            .map_err(|e| AppError::Internal(format!("DB error in reminder query: {}", e)))?;

        let total = rows.len();
        let mut sent = 0u32;
        let mut skipped = 0u32;
        let now = Utc::now();

        for (id_str, email_addr, full_name, due_naive, billing_mode_str, mt_id_opt) in rows {
            let member_id = match Uuid::parse_str(&id_str) {
                Ok(id) => id,
                Err(e) => {
                    tracing::error!("Invalid member id {}: {}", id_str, e);
                    continue;
                }
            };
            let due = chrono::DateTime::<Utc>::from_naive_utc_and_offset(due_naive, Utc);
            let billing_mode = BillingMode::from_str(&billing_mode_str)
                .unwrap_or(BillingMode::Manual);

            // Default card — presence and future validity drive the branch.
            let default_card = self.saved_card_repo
                .find_default_for_member(member_id)
                .await
                .ok()
                .flatten();
            // A card is "good for this renewal" if it's valid through
            // the dues_paid_until date — the moment we'd actually
            // charge. A card valid today but expiring before then
            // still counts as invalid.
            let card_good_at_charge = default_card
                .as_ref()
                .map(|c| c.is_valid_at(due))
                .unwrap_or(false);

            // Billing period of the member's current membership type.
            // Defaults to Yearly if the lookup fails — conservative:
            // we'd rather send a renewal notice than skip silently.
            let billing_period = match mt_id_opt.as_ref().and_then(|s| Uuid::parse_str(s).ok()) {
                Some(mt_id) => self.membership_type_service.get(mt_id).await
                    .ok().flatten()
                    .and_then(|mt| mt.billing_period_enum())
                    .unwrap_or(BillingPeriod::Yearly),
                None => BillingPeriod::Yearly,
            };

            let is_auto_renew = matches!(
                billing_mode,
                BillingMode::CoterieManaged | BillingMode::StripeSubscription
            );

            let due_formatted = due.format("%B %d, %Y").to_string();
            let days_remaining = (due - now).num_days().max(0);

            // Lifetime members shouldn't be in the reminder window to
            // begin with (dues_paid_until is set far in the future),
            // but if one slips through, there's nothing to remind them
            // about. Skip.
            if matches!(billing_period, BillingPeriod::Lifetime) {
                skipped += 1;
                continue;
            }

            // Case 2: auto-renew + card will be valid + short period → skip.
            if is_auto_renew && card_good_at_charge
                && matches!(billing_period, BillingPeriod::Monthly)
            {
                skipped += 1;
                continue;
            }

            // Case 3: auto-renew + card valid + long period → renewal notice.
            if is_auto_renew && card_good_at_charge
                && matches!(billing_period, BillingPeriod::Yearly)
            {
                // Amount display for the renewal notice.
                let amount = match mt_id_opt.as_ref().and_then(|s| Uuid::parse_str(s).ok()) {
                    Some(mt_id) => match self.membership_type_service.get(mt_id).await {
                        Ok(Some(mt)) => format!("${:.2}", mt.fee_cents as f64 / 100.0),
                        _ => "(your membership fee)".to_string(),
                    },
                    None => "(your membership fee)".to_string(),
                };
                // We verified the card is Some in the "card_good_at_charge" branch.
                let card_display = default_card.as_ref()
                    .map(|c| c.display_name())
                    .unwrap_or_else(|| "your card on file".to_string());

                let html = RenewalNoticeHtml {
                    full_name: &full_name, org_name: &org_name,
                    due_date: &due_formatted, days_remaining,
                    amount: &amount, card_display: &card_display,
                    portal_url: &portal_url,
                };
                let text = RenewalNoticeText {
                    full_name: &full_name, org_name: &org_name,
                    due_date: &due_formatted, days_remaining,
                    amount: &amount, card_display: &card_display,
                    portal_url: &portal_url,
                };
                let subject = format!("Your {} membership will renew {}", org_name, due_formatted);
                if self.try_send_and_mark(
                    &id_str, &email_addr, &subject, &html, &text,
                ).await {
                    sent += 1;
                }
                continue;
            }

            // Cases 1 and 4: reminder with optional card-invalid callout.
            // Case 4 = auto-renew but card won't be valid at charge time.
            let card_invalid = is_auto_renew && !card_good_at_charge;

            let html = ReminderHtml {
                full_name: &full_name, org_name: &org_name,
                due_date: &due_formatted, days_remaining,
                pay_url: &pay_url, card_invalid,
            };
            let text = ReminderText {
                full_name: &full_name, org_name: &org_name,
                due_date: &due_formatted, days_remaining,
                pay_url: &pay_url, card_invalid,
            };
            let subject = format!("Your {} dues are due soon", org_name);
            if self.try_send_and_mark(
                &id_str, &email_addr, &subject, &html, &text,
            ).await {
                sent += 1;
            }
        }

        if total > 0 {
            tracing::info!(
                "Dues reminders: {} sent, {} skipped (auto-renew OK) out of {} candidates (window: {} days)",
                sent, skipped, total, reminder_days
            );
        }
        Ok(sent)
    }

    /// Helper: render + send + mark the reminder-sent flag on success.
    /// Returns true if the email went out.
    async fn try_send_and_mark<H, T>(
        &self,
        id_str: &str,
        email_addr: &str,
        subject: &str,
        html: &H,
        text: &T,
    ) -> bool
    where
        H: askama::Template,
        T: askama::Template,
    {
        use crate::email;
        let message = match email::message_from_templates(
            email_addr.to_string(),
            subject.to_string(),
            html,
            text,
        ) {
            Ok(m) => m,
            Err(e) => {
                tracing::error!("Reminder template render failed for {}: {}", id_str, e);
                return false;
            }
        };
        match self.email_sender.send(&message).await {
            Ok(()) => {
                // Mark sent. If the UPDATE fails the email already
                // went out; the next cycle would resend (annoying but
                // not catastrophic) — log loudly so an operator can
                // intervene before the cycle re-runs.
                if let Err(e) = sqlx::query(
                    "UPDATE members SET dues_reminder_sent_at = CURRENT_TIMESTAMP WHERE id = ?"
                )
                .bind(id_str)
                .execute(&self.db_pool)
                .await
                {
                    tracing::error!(
                        "Sent reminder to {} but failed to mark reminder_sent_at — \
                         next cycle may re-send: {}",
                        email_addr, e
                    );
                }
                true
            }
            Err(e) => {
                tracing::warn!("Reminder send failed for {}: {}", email_addr, e);
                false
            }
        }
    }

    /// Slug-based variant of `extend_member_dues`. Convenient for
    /// callers that already have the membership type slug (admin
    /// manual-payment, waive, Stripe checkout success) and don't want
    /// to do a separate slug→id round-trip first.
    pub async fn extend_member_dues_by_slug(
        &self,
        member_id: Uuid,
        membership_type_slug: &str,
    ) -> Result<()> {
        let mt = self.membership_type_service
            .get_by_slug(membership_type_slug)
            .await?
            .ok_or_else(|| AppError::NotFound(format!(
                "Membership type '{}' not found", membership_type_slug
            )))?;
        self.extend_member_dues(member_id, mt.id).await
    }

    pub async fn extend_member_dues(&self, member_id: Uuid, membership_type_id: Uuid) -> Result<()> {
        let membership_type = self
            .membership_type_service
            .get(membership_type_id)
            .await?
            .ok_or_else(|| AppError::NotFound("Membership type not found".to_string()))?;

        let billing_period = membership_type
            .billing_period_enum()
            .unwrap_or(BillingPeriod::Yearly);

        let current_dues: Option<String> = sqlx::query_scalar(
            "SELECT dues_paid_until FROM members WHERE id = ?",
        )
        .bind(member_id.to_string())
        .fetch_optional(&self.db_pool)
        .await
        .map_err(|e| AppError::Internal(format!("Database error: {}", e)))?
        .flatten();

        let now = Utc::now();
        let base_date = current_dues
            .and_then(|s| {
                chrono::DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            })
            .filter(|d| *d > now)
            .unwrap_or(now);

        let new_dues_date = match billing_period {
            BillingPeriod::Monthly => base_date
                .checked_add_months(Months::new(1))
                .unwrap_or(base_date),
            BillingPeriod::Yearly => base_date
                .checked_add_months(Months::new(12))
                .unwrap_or(base_date),
            BillingPeriod::Lifetime => chrono::DateTime::<Utc>::MAX_UTC,
        };

        // Restore Expired -> Active on payment, clear the reminder flag,
        // but don't clobber Suspended (admin-initiated) or Honorary.
        sqlx::query(
            "UPDATE members \
             SET dues_paid_until = ?, \
                 status = CASE WHEN status = 'Expired' THEN 'Active' ELSE status END, \
                 dues_reminder_sent_at = NULL, \
                 updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?",
        )
        .bind(new_dues_date)
        .bind(member_id.to_string())
        .execute(&self.db_pool)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to update dues: {}", e)))?;

        Ok(())
    }

    async fn get_grace_period_days(&self) -> i64 {
        self.settings_service
            .get_number("membership.grace_period_days")
            .await
            .unwrap_or(3)
    }

    async fn get_max_retries(&self) -> i32 {
        self.settings_service
            .get_number("billing.max_retry_attempts")
            .await
            .unwrap_or(3) as i32
    }
}
