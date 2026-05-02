use async_trait::async_trait;
use chrono::{DateTime, Utc, NaiveDateTime};
use sqlx::{SqlitePool, FromRow};
use uuid::Uuid;

use crate::{
    domain::{
        Payer, Payment, PaymentKind, PaymentMethod, PaymentStatus, StripeRef,
        configurable_types::BillingPeriod,
    },
    error::{AppError, Result},
    repository::{MonthlyRevenue, PaymentRepository},
};

#[derive(FromRow)]
struct PaymentRow {
    id: String,
    member_id: Option<String>,
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
    donor_name: Option<String>,
    donor_email: Option<String>,
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
        // The DB CHECK constraint guarantees `member_id IS NOT NULL OR
        // (donor_name AND donor_email)`, so exactly one of the two
        // identity paths is populated. Construct the right `Payer`
        // variant; fail-fast if a row somehow violates the invariant
        // (only possible if the constraint was bypassed by a manual
        // migration). We don't soft-fall-back here — letting a payment
        // through with a fabricated payer would be worse than a 500.
        let member_id = row.member_id
            .as_deref()
            .map(Uuid::parse_str)
            .transpose()
            .map_err(|e| AppError::Database(e.to_string()))?;
        let payer = match (member_id, row.donor_name, row.donor_email) {
            (Some(id), _, _) => Payer::Member(id),
            (None, Some(name), Some(email)) => Payer::PublicDonor { name, email },
            _ => {
                return Err(AppError::Database(format!(
                    "Payment {} has neither member_id nor (donor_name, donor_email) — row violates schema CHECK",
                    row.id,
                )));
            }
        };

        // Tolerate unknown payment_type values from older rows by
        // falling back to Membership (the column default).
        let donation_campaign_id = row.donation_campaign_id
            .as_deref()
            .and_then(|s| Uuid::parse_str(s).ok());
        let kind = match row.payment_type.as_str() {
            "membership" => PaymentKind::Membership,
            "donation" => PaymentKind::Donation { campaign_id: donation_campaign_id },
            "other" => PaymentKind::Other,
            _ => PaymentKind::Membership,
        };

        // Stripe id: parse the prefix into a typed variant. Unknown
        // prefixes (or shapes we no longer recognize) are dropped to
        // `None` rather than panicking — they'll just lose Stripe-
        // side functionality (refund-via-API) until reconciled.
        let external_id = row.stripe_payment_id
            .as_deref()
            .and_then(StripeRef::from_id);

        Ok(Payment {
            id: Uuid::parse_str(&row.id).map_err(|e| AppError::Database(e.to_string()))?,
            payer,
            amount_cents: row.amount_cents,
            currency: row.currency,
            status: Self::parse_payment_status(&row.status)?,
            payment_method: Self::parse_payment_method(&row.payment_method)?,
            kind,
            external_id,
            description: row.description,
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
        let amount_cents_int = payment.amount_cents;
        let status_str = Self::payment_status_to_str(&payment.status);
        let method_str = Self::payment_method_to_str(&payment.payment_method);
        let paid_at_naive = payment.paid_at.map(|dt| dt.naive_utc());
        let now = Utc::now().naive_utc();

        // Decompose the typed Payer / PaymentKind / StripeRef back
        // into the wide DB columns. The schema is unchanged — only
        // the in-memory shape moved to sum types.
        let (member_id_str, donor_name, donor_email) = match &payment.payer {
            Payer::Member(id) => (Some(id.to_string()), None, None),
            Payer::PublicDonor { name, email } => (None, Some(name.clone()), Some(email.clone())),
        };
        let payment_type_str = payment.kind.as_str();
        let donation_campaign_id_str = payment.kind.campaign_id().map(|u| u.to_string());
        let stripe_id_str = payment.external_id.as_ref().map(|r| r.as_str().to_string());

        sqlx::query(
            r#"
            INSERT INTO payments (
                id, member_id, amount_cents, currency, status,
                payment_method, stripe_payment_id, description,
                payment_type, donation_campaign_id,
                donor_name, donor_email,
                paid_at, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#
        )
        .bind(&id_str)
        .bind(&member_id_str)
        .bind(amount_cents_int)
        .bind(&payment.currency)
        .bind(status_str)
        .bind(method_str)
        .bind(&stripe_id_str)
        .bind(&payment.description)
        .bind(payment_type_str)
        .bind(&donation_campaign_id_str)
        .bind(&donor_name)
        .bind(&donor_email)
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
                   donor_name, donor_email,
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
                   donor_name, donor_email,
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
                   donor_name, donor_email,
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
        .bind(payment.member_id().map(|id| id.to_string()))
        .bind(payment.amount_cents)
        .bind(&payment.currency)
        .bind(status_str)
        .bind(method_str)
        .bind(payment.external_id.as_ref().map(|r| r.as_str().to_string()))
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

    async fn mark_refunded(&self, id: Uuid) -> Result<()> {
        sqlx::query(
            "UPDATE payments SET status = 'Refunded', updated_at = ? WHERE id = ?",
        )
        .bind(Utc::now().naive_utc())
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

    async fn revenue_by_month(&self, months_back: u32) -> Result<Vec<MonthlyRevenue>> {
        // SQLite-friendly: strftime extracts year/month; we filter on
        // paid_at being non-null AND status='Completed' so refunded /
        // pending / failed rows don't pollute the totals. The cutoff
        // is `now - months_back months`, computed at the DB level so
        // all timestamps stay UTC.
        //
        // Result is ordered newest-month first; the dashboard
        // presents months top-down. payment_type comes back as the
        // raw lowercase string and is stored on `MonthlyRevenue`
        // as-is — see the doc on that struct for why.
        let cutoff_months = months_back as i64;
        let rows: Vec<(String, String, String, i64, i64)> = sqlx::query_as(
            r#"
            SELECT
                strftime('%Y', paid_at)        AS year_str,
                strftime('%m', paid_at)        AS month_str,
                payment_type                    AS payment_type,
                SUM(amount_cents)               AS total_cents,
                COUNT(*)                        AS payment_count
            FROM payments
            WHERE status = 'Completed'
              AND paid_at IS NOT NULL
              AND paid_at >= datetime('now', ?)
            GROUP BY year_str, month_str, payment_type
            ORDER BY year_str DESC, month_str DESC, payment_type ASC
            "#,
        )
        .bind(format!("-{} months", cutoff_months))
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        let mut out = Vec::with_capacity(rows.len());
        for (year_str, month_str, type_str, total, count) in rows {
            let year: i32 = year_str.parse()
                .map_err(|e: std::num::ParseIntError| AppError::Database(e.to_string()))?;
            let month: u32 = month_str.parse()
                .map_err(|e: std::num::ParseIntError| AppError::Database(e.to_string()))?;
            out.push(MonthlyRevenue {
                year,
                month,
                payment_type: type_str,
                total_cents: total,
                payment_count: count,
            });
        }
        Ok(out)
    }
}