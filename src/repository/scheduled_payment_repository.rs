use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

use crate::{
    domain::{ScheduledPayment, ScheduledPaymentStatus},
    error::{AppError, Result},
    repository::ScheduledPaymentRepository,
};

#[derive(FromRow)]
struct ScheduledPaymentRow {
    id: String,
    member_id: String,
    membership_type_id: String,
    amount_cents: i64,
    currency: String,
    due_date: String,
    status: String,
    retry_count: i32,
    last_attempt_at: Option<NaiveDateTime>,
    payment_id: Option<String>,
    failure_reason: Option<String>,
    created_at: NaiveDateTime,
    updated_at: NaiveDateTime,
}

pub struct SqliteScheduledPaymentRepository {
    pool: SqlitePool,
}

impl SqliteScheduledPaymentRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    fn row_to_scheduled_payment(row: ScheduledPaymentRow) -> Result<ScheduledPayment> {
        let status = ScheduledPaymentStatus::from_str(&row.status)
            .ok_or_else(|| AppError::Database(format!("Invalid status: {}", row.status)))?;

        let due_date = NaiveDate::parse_from_str(&row.due_date, "%Y-%m-%d")
            .map_err(|e| AppError::Database(format!("Invalid due_date: {}", e)))?;

        Ok(ScheduledPayment {
            id: Uuid::parse_str(&row.id).map_err(|e| AppError::Database(e.to_string()))?,
            member_id: Uuid::parse_str(&row.member_id)
                .map_err(|e| AppError::Database(e.to_string()))?,
            membership_type_id: Uuid::parse_str(&row.membership_type_id)
                .map_err(|e| AppError::Database(e.to_string()))?,
            amount_cents: row.amount_cents,
            currency: row.currency,
            due_date,
            status,
            retry_count: row.retry_count,
            last_attempt_at: row
                .last_attempt_at
                .map(|dt| DateTime::from_naive_utc_and_offset(dt, Utc)),
            payment_id: row
                .payment_id
                .map(|s| Uuid::parse_str(&s))
                .transpose()
                .map_err(|e| AppError::Database(e.to_string()))?,
            failure_reason: row.failure_reason,
            created_at: DateTime::from_naive_utc_and_offset(row.created_at, Utc),
            updated_at: DateTime::from_naive_utc_and_offset(row.updated_at, Utc),
        })
    }
}

#[async_trait]
impl ScheduledPaymentRepository for SqliteScheduledPaymentRepository {
    async fn create(&self, payment: ScheduledPayment) -> Result<ScheduledPayment> {
        let id_str = payment.id.to_string();
        let member_id_str = payment.member_id.to_string();
        let membership_type_id_str = payment.membership_type_id.to_string();
        let due_date_str = payment.due_date.format("%Y-%m-%d").to_string();
        let status_str = payment.status.as_str();
        let last_attempt_at_naive = payment.last_attempt_at.map(|dt| dt.naive_utc());
        let payment_id_str = payment.payment_id.map(|id| id.to_string());
        let now = Utc::now().naive_utc();

        sqlx::query(
            r#"
            INSERT INTO scheduled_payments (
                id, member_id, membership_type_id, amount_cents, currency,
                due_date, status, retry_count, last_attempt_at, payment_id,
                failure_reason, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&id_str)
        .bind(&member_id_str)
        .bind(&membership_type_id_str)
        .bind(payment.amount_cents)
        .bind(&payment.currency)
        .bind(&due_date_str)
        .bind(status_str)
        .bind(payment.retry_count)
        .bind(last_attempt_at_naive)
        .bind(&payment_id_str)
        .bind(&payment.failure_reason)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        self.find_by_id(payment.id).await?.ok_or_else(|| {
            AppError::Database("Failed to retrieve created scheduled payment".to_string())
        })
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<ScheduledPayment>> {
        let id_str = id.to_string();
        let row = sqlx::query_as::<_, ScheduledPaymentRow>(
            r#"
            SELECT id, member_id, membership_type_id, amount_cents, currency,
                   due_date, status, retry_count, last_attempt_at, payment_id,
                   failure_reason, created_at, updated_at
            FROM scheduled_payments
            WHERE id = ?
            "#,
        )
        .bind(id_str)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        match row {
            Some(r) => Ok(Some(Self::row_to_scheduled_payment(r)?)),
            None => Ok(None),
        }
    }

    async fn find_by_member(&self, member_id: Uuid) -> Result<Vec<ScheduledPayment>> {
        let member_id_str = member_id.to_string();
        let rows = sqlx::query_as::<_, ScheduledPaymentRow>(
            r#"
            SELECT id, member_id, membership_type_id, amount_cents, currency,
                   due_date, status, retry_count, last_attempt_at, payment_id,
                   failure_reason, created_at, updated_at
            FROM scheduled_payments
            WHERE member_id = ?
            ORDER BY due_date DESC
            "#,
        )
        .bind(member_id_str)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        rows.into_iter()
            .map(Self::row_to_scheduled_payment)
            .collect()
    }

    async fn find_pending_due_before(&self, date: NaiveDate) -> Result<Vec<ScheduledPayment>> {
        let date_str = date.format("%Y-%m-%d").to_string();
        let rows = sqlx::query_as::<_, ScheduledPaymentRow>(
            r#"
            SELECT id, member_id, membership_type_id, amount_cents, currency,
                   due_date, status, retry_count, last_attempt_at, payment_id,
                   failure_reason, created_at, updated_at
            FROM scheduled_payments
            WHERE status = 'pending' AND due_date <= ?
            ORDER BY due_date ASC
            "#,
        )
        .bind(date_str)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        rows.into_iter()
            .map(Self::row_to_scheduled_payment)
            .collect()
    }

    async fn update_status(
        &self,
        id: Uuid,
        status: ScheduledPaymentStatus,
        failure_reason: Option<String>,
    ) -> Result<ScheduledPayment> {
        let id_str = id.to_string();
        let status_str = status.as_str();
        let now = Utc::now().naive_utc();

        sqlx::query(
            r#"
            UPDATE scheduled_payments
            SET status = ?, failure_reason = ?, last_attempt_at = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(status_str)
        .bind(&failure_reason)
        .bind(now)
        .bind(now)
        .bind(&id_str)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        self.find_by_id(id).await?.ok_or_else(|| {
            AppError::Database("Failed to retrieve updated scheduled payment".to_string())
        })
    }

    async fn increment_retry(&self, id: Uuid) -> Result<ScheduledPayment> {
        let id_str = id.to_string();
        let now = Utc::now().naive_utc();

        sqlx::query(
            r#"
            UPDATE scheduled_payments
            SET retry_count = retry_count + 1, last_attempt_at = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(now)
        .bind(now)
        .bind(&id_str)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        self.find_by_id(id).await?.ok_or_else(|| {
            AppError::Database("Failed to retrieve updated scheduled payment".to_string())
        })
    }

    async fn link_payment(&self, id: Uuid, payment_id: Uuid) -> Result<ScheduledPayment> {
        let id_str = id.to_string();
        let payment_id_str = payment_id.to_string();
        let now = Utc::now().naive_utc();

        sqlx::query(
            r#"
            UPDATE scheduled_payments
            SET payment_id = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(&payment_id_str)
        .bind(now)
        .bind(&id_str)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        self.find_by_id(id).await?.ok_or_else(|| {
            AppError::Database("Failed to retrieve updated scheduled payment".to_string())
        })
    }

    async fn list_failures_since(
        &self,
        since: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<ScheduledPayment>> {
        // Filter on `last_attempt_at` rather than `updated_at` so the
        // window matches "when the failure actually happened" — the
        // billing runner stamps last_attempt_at on every attempt.
        // Rows whose last_attempt_at is null shouldn't exist with
        // status='failed' (the runner always stamps before failing),
        // but we exclude them defensively.
        let rows = sqlx::query_as::<_, ScheduledPaymentRow>(
            r#"
            SELECT id, member_id, membership_type_id, amount_cents, currency,
                   due_date, status, retry_count, last_attempt_at, payment_id,
                   failure_reason, created_at, updated_at
            FROM scheduled_payments
            WHERE status = 'failed'
              AND last_attempt_at IS NOT NULL
              AND last_attempt_at >= ?
            ORDER BY last_attempt_at DESC
            "#,
        )
        .bind(since.naive_utc())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        rows.into_iter()
            .map(Self::row_to_scheduled_payment)
            .collect()
    }
}
