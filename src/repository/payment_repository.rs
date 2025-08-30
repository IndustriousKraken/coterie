use async_trait::async_trait;
use chrono::{DateTime, Utc, NaiveDateTime};
use sqlx::{SqlitePool, FromRow};
use uuid::Uuid;

use crate::{
    domain::{Payment, PaymentStatus, PaymentMethod},
    error::{AppError, Result},
    repository::PaymentRepository,
};

#[derive(FromRow)]
struct PaymentRow {
    id: String,
    member_id: String,
    amount_cents: i32,
    currency: String,
    status: String,
    payment_method: String,
    stripe_payment_id: Option<String>,
    description: String,
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
        Ok(Payment {
            id: Uuid::parse_str(&row.id).map_err(|e| AppError::Database(e.to_string()))?,
            member_id: Uuid::parse_str(&row.member_id).map_err(|e| AppError::Database(e.to_string()))?,
            amount_cents: row.amount_cents as i64,
            currency: row.currency,
            status: Self::parse_payment_status(&row.status)?,
            payment_method: Self::parse_payment_method(&row.payment_method)?,
            stripe_payment_id: row.stripe_payment_id,
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
        let member_id_str = payment.member_id.to_string();
        let amount_cents_int = payment.amount_cents as i32;
        let status_str = Self::payment_status_to_str(&payment.status);
        let method_str = Self::payment_method_to_str(&payment.payment_method);
        let paid_at_naive = payment.paid_at.map(|dt| dt.naive_utc());
        let now = Utc::now().naive_utc();

        sqlx::query(
            r#"
            INSERT INTO payments (
                id, member_id, amount_cents, currency, status,
                payment_method, stripe_payment_id, description,
                paid_at, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
        .bind(payment.amount_cents as i32)
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