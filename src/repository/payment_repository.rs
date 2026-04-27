use async_trait::async_trait;
use chrono::{DateTime, Utc, NaiveDateTime};
use sqlx::{SqlitePool, FromRow};
use uuid::Uuid;

use crate::{
    domain::{Payment, PaymentStatus, PaymentMethod, PaymentType, configurable_types::BillingPeriod},
    error::{AppError, Result},
    repository::PaymentRepository,
};

#[derive(FromRow)]
struct PaymentRow {
    id: String,
    member_id: String,
    /// SQLite INTEGER is up to 8 bytes; using i64 here matches both
    /// the schema's actual storage and the domain's `Payment.amount_cents`.
    /// The previous i32 silently truncated values >$21.5M cents.
    amount_cents: i64,
    currency: String,
    status: String,
    payment_method: String,
    stripe_payment_id: Option<String>,
    description: String,
    payment_type: String,
    donation_campaign_id: Option<String>,
    paid_at: Option<NaiveDateTime>,
    created_at: NaiveDateTime,
    updated_at: NaiveDateTime,
}

pub struct SqlitePaymentRepository {
    pool: SqlitePool,
}

impl SqlitePaymentRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    fn row_to_payment(row: PaymentRow) -> Result<Payment> {
        // Tolerate unknown payment_type values from older rows by
        // falling back to Membership (the column default) — that
        // matches what the row was actually doing pre-migration.
        let payment_type = PaymentType::from_str(&row.payment_type)
            .unwrap_or(PaymentType::Membership);
        let donation_campaign_id = row.donation_campaign_id
            .as_deref()
            .and_then(|s| Uuid::parse_str(s).ok());
        Ok(Payment {
            id: Uuid::parse_str(&row.id).map_err(|e| AppError::Database(e.to_string()))?,
            member_id: Uuid::parse_str(&row.member_id).map_err(|e| AppError::Database(e.to_string()))?,
            amount_cents: row.amount_cents,
            currency: row.currency,
            status: Self::parse_payment_status(&row.status)?,
            payment_method: Self::parse_payment_method(&row.payment_method)?,
            stripe_payment_id: row.stripe_payment_id,
            description: row.description,
            payment_type,
            donation_campaign_id,
            paid_at: row.paid_at.map(|dt| DateTime::from_naive_utc_and_offset(dt, Utc)),
            created_at: DateTime::from_naive_utc_and_offset(row.created_at, Utc),
            updated_at: DateTime::from_naive_utc_and_offset(row.updated_at, Utc),
        })
    }

    fn parse_payment_status(s: &str) -> Result<PaymentStatus> {
        match s {
            "Pending" => Ok(PaymentStatus::Pending),
            "Completed" => Ok(PaymentStatus::Completed),
            "Failed" => Ok(PaymentStatus::Failed),
            "Refunded" => Ok(PaymentStatus::Refunded),
            _ => Err(AppError::Database(format!("Invalid payment status: {}", s))),
        }
    }

    fn payment_status_to_str(status: &PaymentStatus) -> &'static str {
        match status {
            PaymentStatus::Pending => "Pending",
            PaymentStatus::Completed => "Completed",
            PaymentStatus::Failed => "Failed",
            PaymentStatus::Refunded => "Refunded",
        }
    }

    fn parse_payment_method(s: &str) -> Result<PaymentMethod> {
        match s {
            "Stripe" => Ok(PaymentMethod::Stripe),
            "Manual" => Ok(PaymentMethod::Manual),
            "Waived" => Ok(PaymentMethod::Waived),
            _ => Err(AppError::Database(format!("Invalid payment method: {}", s))),
        }
    }

    fn payment_method_to_str(method: &PaymentMethod) -> &'static str {
        match method {
            PaymentMethod::Stripe => "Stripe",
            PaymentMethod::Manual => "Manual",
            PaymentMethod::Waived => "Waived",
        }
    }
}

#[async_trait]
impl PaymentRepository for SqlitePaymentRepository {
    async fn create(&self, payment: Payment) -> Result<Payment> {
        let id_str = payment.id.to_string();
        let member_id_str = payment.member_id.to_string();
        let amount_cents_int = payment.amount_cents;
        let status_str = Self::payment_status_to_str(&payment.status);
        let method_str = Self::payment_method_to_str(&payment.payment_method);
        let paid_at_naive = payment.paid_at.map(|dt| dt.naive_utc());
        let now = Utc::now().naive_utc();

        let payment_type_str = payment.payment_type.as_str();
        let donation_campaign_id_str = payment.donation_campaign_id.map(|u| u.to_string());

        sqlx::query(
            r#"
            INSERT INTO payments (
                id, member_id, amount_cents, currency, status,
                payment_method, stripe_payment_id, description,
                payment_type, donation_campaign_id,
                paid_at, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#
        )
        .bind(&id_str)
        .bind(&member_id_str)
        .bind(amount_cents_int)
        .bind(&payment.currency)
        .bind(status_str)
        .bind(method_str)
        .bind(&payment.stripe_payment_id)
        .bind(&payment.description)
        .bind(payment_type_str)
        .bind(&donation_campaign_id_str)
        .bind(paid_at_naive)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        self.find_by_id(payment.id).await?.ok_or_else(|| {
            AppError::Database("Failed to retrieve created payment".to_string())
        })
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<Payment>> {
        let id_str = id.to_string();
        let row = sqlx::query_as::<_, PaymentRow>(
            r#"
            SELECT id, member_id, amount_cents, currency, status,
                   payment_method, stripe_payment_id, description,
                   payment_type, donation_campaign_id,
                   paid_at, created_at, updated_at
            FROM payments
            WHERE id = ?
            "#
        )
        .bind(id_str)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        match row {
            Some(r) => Ok(Some(Self::row_to_payment(r)?)),
            None => Ok(None)
        }
    }

    async fn find_by_member(&self, member_id: Uuid) -> Result<Vec<Payment>> {
        let member_id_str = member_id.to_string();
        let rows = sqlx::query_as::<_, PaymentRow>(
            r#"
            SELECT id, member_id, amount_cents, currency, status,
                   payment_method, stripe_payment_id, description,
                   payment_type, donation_campaign_id,
                   paid_at, created_at, updated_at
            FROM payments
            WHERE member_id = ?
            ORDER BY created_at DESC
            "#
        )
        .bind(member_id_str)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        rows.into_iter()
            .map(Self::row_to_payment)
            .collect()
    }

    async fn find_by_stripe_id(&self, stripe_id: &str) -> Result<Option<Payment>> {
        let row = sqlx::query_as::<_, PaymentRow>(
            r#"
            SELECT id, member_id, amount_cents, currency, status,
                   payment_method, stripe_payment_id, description,
                   payment_type, donation_campaign_id,
                   paid_at, created_at, updated_at
            FROM payments
            WHERE stripe_payment_id = ?
            "#
        )
        .bind(stripe_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        match row {
            Some(r) => Ok(Some(Self::row_to_payment(r)?)),
            None => Ok(None)
        }
    }

    async fn update(&self, id: Uuid, payment: Payment) -> Result<Payment> {
        let id_str = id.to_string();
        let now = Utc::now().naive_utc();
        let status_str = Self::payment_status_to_str(&payment.status);
        let method_str = Self::payment_method_to_str(&payment.payment_method);
        let paid_at_naive = payment.paid_at.map(|dt| dt.naive_utc());

        sqlx::query(
            r#"
            UPDATE payments
            SET member_id = ?,
                amount_cents = ?,
                currency = ?,
                status = ?,
                payment_method = ?,
                stripe_payment_id = ?,
                description = ?,
                paid_at = ?,
                updated_at = ?
            WHERE id = ?
            "#
        )
        .bind(payment.member_id.to_string())
        .bind(payment.amount_cents)
        .bind(&payment.currency)
        .bind(status_str)
        .bind(method_str)
        .bind(&payment.stripe_payment_id)
        .bind(&payment.description)
        .bind(paid_at_naive)
        .bind(now)
        .bind(&id_str)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        self.find_by_id(id).await?.ok_or_else(|| {
            AppError::Database("Failed to retrieve updated payment".to_string())
        })
    }

    async fn complete_pending_payment(
        &self,
        id: Uuid,
        stripe_payment_id: &str,
    ) -> Result<bool> {
        let now = Utc::now().naive_utc();
        let res = sqlx::query(
            "UPDATE payments \
             SET status = 'Completed', \
                 stripe_payment_id = ?, \
                 paid_at = ?, \
                 updated_at = ? \
             WHERE id = ? AND status = 'Pending'",
        )
        .bind(stripe_payment_id)
        .bind(now)
        .bind(now)
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(res.rows_affected() == 1)
    }

    async fn fail_pending_payment(&self, id: Uuid) -> Result<bool> {
        let now = Utc::now().naive_utc();
        let res = sqlx::query(
            "UPDATE payments \
             SET status = 'Failed', updated_at = ? \
             WHERE id = ? AND status = 'Pending'",
        )
        .bind(now)
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(res.rows_affected() == 1)
    }

    async fn claim_payment_for_refund(&self, id: Uuid) -> Result<bool> {
        let now = Utc::now().naive_utc();
        let res = sqlx::query(
            "UPDATE payments \
             SET status = 'Refunded', updated_at = ? \
             WHERE id = ? AND status = 'Completed'",
        )
        .bind(now)
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(res.rows_affected() == 1)
    }

    async fn unclaim_refund(&self, id: Uuid) -> Result<()> {
        let now = Utc::now().naive_utc();
        sqlx::query(
            "UPDATE payments \
             SET status = 'Completed', updated_at = ? \
             WHERE id = ? AND status = 'Refunded'",
        )
        .bind(now)
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    async fn extend_dues_for_payment_atomic(
        &self,
        payment_id: Uuid,
        member_id: Uuid,
        billing_period: BillingPeriod,
    ) -> Result<bool> {
        use chrono::Months;

        let mut tx = self.pool.begin().await
            .map_err(|e| AppError::Database(e.to_string()))?;

        // Atomic claim. dues_extended_at is the per-payment idempotency
        // anchor: only the first caller for this payment_id sees
        // rows_affected == 1; any later caller (including a webhook
        // retry after rollback) sees 0 and no-ops below.
        let now_naive = Utc::now().naive_utc();
        let claim = sqlx::query(
            "UPDATE payments SET dues_extended_at = ? \
             WHERE id = ? AND dues_extended_at IS NULL",
        )
        .bind(now_naive)
        .bind(payment_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        if claim.rows_affected() == 0 {
            tx.commit().await.map_err(|e| AppError::Database(e.to_string()))?;
            return Ok(false);
        }

        // Read current dues INSIDE the transaction so SQLite's write
        // lock serializes us against any concurrent payment for the
        // same member. Without the txn, two payments could both read
        // D and both write D+1y, losing one period.
        let current_dues: Option<DateTime<Utc>> = sqlx::query_scalar::<_, Option<DateTime<Utc>>>(
            "SELECT dues_paid_until FROM members WHERE id = ?",
        )
        .bind(member_id.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?
        .flatten();

        let now_utc = Utc::now();
        let base_date = current_dues.filter(|d| *d > now_utc).unwrap_or(now_utc);
        let new_dues_date = match billing_period {
            BillingPeriod::Monthly => base_date.checked_add_months(Months::new(1)).unwrap_or(base_date),
            BillingPeriod::Yearly => base_date.checked_add_months(Months::new(12)).unwrap_or(base_date),
            BillingPeriod::Lifetime => DateTime::<Utc>::MAX_UTC,
        };

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
        .execute(&mut *tx)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        tx.commit().await.map_err(|e| AppError::Database(e.to_string()))?;
        Ok(true)
    }

    async fn update_status(&self, id: Uuid, status: PaymentStatus) -> Result<Payment> {
        let id_str = id.to_string();
        let status_str = Self::payment_status_to_str(&status);
        let now = Utc::now().naive_utc();
        
        // If status is completed, also update paid_at
        let paid_at_naive = if status == PaymentStatus::Completed {
            Some(now)
        } else {
            None
        };

        sqlx::query(
            r#"
            UPDATE payments
            SET status = ?, 
                paid_at = COALESCE(?, paid_at),
                updated_at = ?
            WHERE id = ?
            "#
        )
        .bind(status_str)
        .bind(paid_at_naive)
        .bind(now)
        .bind(&id_str)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        self.find_by_id(id).await?.ok_or_else(|| {
            AppError::Database("Failed to retrieve updated payment".to_string())
        })
    }
}