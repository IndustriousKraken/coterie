use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A saved payment method (credit/debit card) for a member
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SavedCard {
    pub id: Uuid,
    pub member_id: Uuid,
    pub stripe_payment_method_id: String,
    pub card_last_four: String,
    pub card_brand: String,
    pub exp_month: i32,
    pub exp_year: i32,
    pub is_default: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl SavedCard {
    /// Check if the card is expired
    pub fn is_expired(&self) -> bool {
        let now = Utc::now();
        let current_year = now.format("%Y").to_string().parse::<i32>().unwrap_or(0);
        let current_month = now.format("%m").to_string().parse::<i32>().unwrap_or(0);

        self.exp_year < current_year ||
            (self.exp_year == current_year && self.exp_month < current_month)
    }

    /// Display string like "Visa •••• 4242"
    pub fn display_name(&self) -> String {
        let brand = match self.card_brand.as_str() {
            "visa" => "Visa",
            "mastercard" => "Mastercard",
            "amex" => "Amex",
            "discover" => "Discover",
            other => other,
        };
        format!("{} •••• {}", brand, self.card_last_four)
    }

    /// Expiration display like "12/25"
    pub fn exp_display(&self) -> String {
        format!("{:02}/{:02}", self.exp_month, self.exp_year % 100)
    }
}

/// How a member's recurring billing is handled
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BillingMode {
    /// No automatic billing; member pays manually or admin records payments
    #[default]
    Manual,
    /// Coterie schedules and initiates charges using saved payment method
    CoterieManaged,
    /// Legacy: Stripe manages the subscription, we just listen to webhooks
    StripeSubscription,
}

impl BillingMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            BillingMode::Manual => "manual",
            BillingMode::CoterieManaged => "coterie_managed",
            BillingMode::StripeSubscription => "stripe_subscription",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "manual" => Some(BillingMode::Manual),
            "coterie_managed" => Some(BillingMode::CoterieManaged),
            "stripe_subscription" => Some(BillingMode::StripeSubscription),
            _ => None,
        }
    }
}

// SQLx type conversion
impl<'r> sqlx::Decode<'r, sqlx::Sqlite> for BillingMode {
    fn decode(value: sqlx::sqlite::SqliteValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        let s = <String as sqlx::Decode<sqlx::Sqlite>>::decode(value)?;
        BillingMode::from_str(&s).ok_or_else(|| format!("Invalid billing mode: {}", s).into())
    }
}

impl sqlx::Type<sqlx::Sqlite> for BillingMode {
    fn type_info() -> sqlx::sqlite::SqliteTypeInfo {
        <String as sqlx::Type<sqlx::Sqlite>>::type_info()
    }

    fn compatible(ty: &sqlx::sqlite::SqliteTypeInfo) -> bool {
        <String as sqlx::Type<sqlx::Sqlite>>::compatible(ty)
    }
}

impl<'q> sqlx::Encode<'q, sqlx::Sqlite> for BillingMode {
    fn encode_by_ref(&self, args: &mut Vec<sqlx::sqlite::SqliteArgumentValue<'q>>) -> sqlx::encode::IsNull {
        <String as sqlx::Encode<'q, sqlx::Sqlite>>::encode_by_ref(&self.as_str().to_string(), args)
    }
}
