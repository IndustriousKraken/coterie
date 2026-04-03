use chrono::{Months, NaiveDate, Utc};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    domain::{
        configurable_types::BillingPeriod, BillingMode, Payment, PaymentMethod, PaymentStatus,
        ScheduledPayment, ScheduledPaymentStatus,
    },
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
    stripe_client: Option<Arc<StripeClient>>,
    db_pool: SqlitePool,
}

impl BillingService {
    pub fn new(
        scheduled_payment_repo: Arc<dyn ScheduledPaymentRepository>,
        payment_repo: Arc<dyn PaymentRepository>,
        saved_card_repo: Arc<dyn SavedCardRepository>,
        membership_type_service: Arc<MembershipTypeService>,
        settings_service: Arc<SettingsService>,
        stripe_client: Option<Arc<StripeClient>>,
        db_pool: SqlitePool,
    ) -> Self {
        Self {
            scheduled_payment_repo,
            payment_repo,
            saved_card_repo,
            membership_type_service,
            settings_service,
            stripe_client,
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

        // Attempt the charge
        match stripe_client
            .charge_saved_card(
                sp.member_id,
                &card.stripe_payment_method_id,
                sp.amount_cents,
                &description,
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

                // Schedule next renewal
                if let Some(mt) = &membership_type {
                    let _ = self.schedule_renewal(sp.member_id, &mt.slug).await;
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

    /// Check for members past grace period and expire them.
    pub async fn check_expired_members(&self) -> Result<u32> {
        let grace_days = self.get_grace_period_days().await;

        // Find active members whose dues_paid_until + grace period has passed
        let expired_count = sqlx::query(
            r#"
            UPDATE members
            SET status = 'Expired', updated_at = CURRENT_TIMESTAMP
            WHERE status = 'Active'
              AND dues_paid_until IS NOT NULL
              AND date(dues_paid_until, '+' || ? || ' days') < date('now')
              AND bypass_dues = 0
            "#,
        )
        .bind(grace_days)
        .execute(&self.db_pool)
        .await
        .map_err(|e| AppError::Internal(format!("Database error: {}", e)))?
        .rows_affected() as u32;

        if expired_count > 0 {
            tracing::info!(
                "Expired {} members past grace period ({} days)",
                expired_count,
                grace_days
            );
        }

        Ok(expired_count)
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

        sqlx::query(
            "UPDATE members SET dues_paid_until = ?, status = 'Active', updated_at = CURRENT_TIMESTAMP WHERE id = ?",
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
