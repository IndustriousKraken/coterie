use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A scheduled payment for Coterie-managed billing
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ScheduledPayment {
    pub id: Uuid,
    pub member_id: Uuid,
    pub membership_type_id: Uuid,
    pub amount_cents: i64,
    pub currency: String,
    pub due_date: NaiveDate,
    pub status: ScheduledPaymentStatus,
    pub retry_count: i32,
    pub last_attempt_at: Option<DateTime<Utc>>,
    pub payment_id: Option<Uuid>,
    pub failure_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Status of a scheduled payment
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledPaymentStatus {
    /// Waiting to be processed
    #[default]
    Pending,
    /// Currently being processed (charge in flight)
    Processing,
    /// Successfully charged
    Completed,
    /// Failed after all retries exhausted
    Failed,
    /// Manually canceled (e.g., member downgraded or left)
    Canceled,
}

impl ScheduledPaymentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ScheduledPaymentStatus::Pending => "pending",
            ScheduledPaymentStatus::Processing => "processing",
            ScheduledPaymentStatus::Completed => "completed",
            ScheduledPaymentStatus::Failed => "failed",
            ScheduledPaymentStatus::Canceled => "canceled",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(ScheduledPaymentStatus::Pending),
            "processing" => Some(ScheduledPaymentStatus::Processing),
            "completed" => Some(ScheduledPaymentStatus::Completed),
            "failed" => Some(ScheduledPaymentStatus::Failed),
            "canceled" => Some(ScheduledPaymentStatus::Canceled),
            _ => None,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Canceled)
    }
}

// SQLx type conversion
impl<'r> sqlx::Decode<'r, sqlx::Sqlite> for ScheduledPaymentStatus {
    fn decode(value: sqlx::sqlite::SqliteValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        let s = <String as sqlx::Decode<sqlx::Sqlite>>::decode(value)?;
        ScheduledPaymentStatus::from_str(&s)
            .ok_or_else(|| format!("Invalid scheduled payment status: {}", s).into())
    }
}

impl sqlx::Type<sqlx::Sqlite> for ScheduledPaymentStatus {
    fn type_info() -> sqlx::sqlite::SqliteTypeInfo {
        <String as sqlx::Type<sqlx::Sqlite>>::type_info()
    }

    fn compatible(ty: &sqlx::sqlite::SqliteTypeInfo) -> bool {
        <String as sqlx::Type<sqlx::Sqlite>>::compatible(ty)
    }
}

impl<'q> sqlx::Encode<'q, sqlx::Sqlite> for ScheduledPaymentStatus {
    fn encode_by_ref(&self, args: &mut Vec<sqlx::sqlite::SqliteArgumentValue<'q>>) -> sqlx::encode::IsNull {
        <String as sqlx::Encode<'q, sqlx::Sqlite>>::encode_by_ref(&self.as_str().to_string(), args)
    }
}
