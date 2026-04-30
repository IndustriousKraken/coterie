//! Member-facing billing notifications + admin-side alerts. Pure
//! rendering + dispatch; no payment-state mutation. Used by:
//! - `WebhookDispatcher` for the two Stripe-subscription notify paths
//!   (`notify_subscription_cancelled`, `notify_subscription_payment_failed`)
//! - The daily reminder runner (`send_dues_reminders`)
//!
//! Split out of the original `BillingService` so the email-template
//! and AdminAlert plumbing has its own home, separate from the auto-
//! renew lifecycle and expiration sweeps that share none of its deps.

use chrono::Utc;
use sqlx::SqlitePool;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    email::EmailSender,
    error::{AppError, Result},
    integrations::{IntegrationEvent, IntegrationManager},
    repository::{MemberRepository, SavedCardRepository},
    service::{membership_type_service::MembershipTypeService, settings_service::SettingsService},
};

pub struct Notifications {
    member_repo: Arc<dyn MemberRepository>,
    saved_card_repo: Arc<dyn SavedCardRepository>,
    membership_type_service: Arc<MembershipTypeService>,
    settings_service: Arc<SettingsService>,
    email_sender: Arc<dyn EmailSender>,
    integration_manager: Arc<IntegrationManager>,
    /// Absolute URL to this Coterie instance — used to build links in
    /// outgoing reminder emails. Comes from ServerConfig::base_url.
    base_url: String,
    /// Held for the dues-reminder candidate query, which still uses
    /// raw SQL pending a typed `find_pending_dues_reminders` repo
    /// method. F1 deferred this site.
    db_pool: SqlitePool,
}

impl Notifications {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        member_repo: Arc<dyn MemberRepository>,
        saved_card_repo: Arc<dyn SavedCardRepository>,
        membership_type_service: Arc<MembershipTypeService>,
        settings_service: Arc<SettingsService>,
        email_sender: Arc<dyn EmailSender>,
        integration_manager: Arc<IntegrationManager>,
        base_url: String,
        db_pool: SqlitePool,
    ) -> Self {
        Self {
            member_repo,
            saved_card_repo,
            membership_type_service,
            settings_service,
            email_sender,
            integration_manager,
            base_url,
            db_pool,
        }
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
            email::templates::{ReminderHtml, ReminderText, RenewalNoticeHtml, RenewalNoticeText},
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
                if let Ok(member_uuid) = Uuid::parse_str(id_str) {
                    if let Err(e) = self.member_repo.set_dues_reminder_sent(member_uuid).await {
                        tracing::error!(
                            "Sent reminder to {} but failed to mark reminder_sent_at — \
                             next cycle may re-send: {}",
                            email_addr, e
                        );
                    }
                }
                true
            }
            Err(e) => {
                tracing::warn!("Reminder send failed for {}: {}", email_addr, e);
                false
            }
        }
    }
}
