use chrono::{Months, NaiveDate, Utc};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    domain::{
        configurable_types::BillingPeriod, BillingMode, Payment, PaymentMethod, PaymentStatus,
        ScheduledPayment, ScheduledPaymentStatus,
    },
    email::EmailSender,
    error::{AppError, Result},
    payments::StripeClient,
    repository::{PaymentRepository, SavedCardRepository, ScheduledPaymentRepository},
    service::{membership_type_service::MembershipTypeService, settings_service::SettingsService},
};
use sqlx::SqlitePool;

pub struct BillingService {
    scheduled_payment_repo: Arc<dyn ScheduledPaymentRepository>,
    payment_repo: Arc<dyn PaymentRepository>,
    saved_card_repo: Arc<dyn SavedCardRepository>,
    membership_type_service: Arc<MembershipTypeService>,
    settings_service: Arc<SettingsService>,
    email_sender: Arc<dyn EmailSender>,
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
        membership_type_service: Arc<MembershipTypeService>,
        settings_service: Arc<SettingsService>,
        email_sender: Arc<dyn EmailSender>,
        stripe_client: Option<Arc<StripeClient>>,
        base_url: String,
        db_pool: SqlitePool,
    ) -> Self {
        Self {
            scheduled_payment_repo,
            payment_repo,
            saved_card_repo,
            membership_type_service,
            settings_service,
            email_sender,
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

    async fn extend_member_dues(&self, member_id: Uuid, membership_type_id: Uuid) -> Result<()> {
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
