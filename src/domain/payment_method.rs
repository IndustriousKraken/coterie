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
    /// Whether the card is valid at a specific point in time. Cards are
    /// valid through the END of their `exp_month` (industry convention),
    /// so a card with exp 05/2026 is valid on any day in May 2026 but
    /// invalid on 1 June 2026.
    ///
    /// Use this with the expected charge date — not just "now" — to
    /// catch cards that will have expired by the time we try to charge.
    pub fn is_valid_at(&self, when: DateTime<Utc>) -> bool {
        use chrono::Datelike;
        let year = when.year();
        let month = when.month() as i32;
        self.exp_year > year || (self.exp_year == year && self.exp_month >= month)
    }

    /// Convenience: whether the card is valid right now.
    pub fn is_expired(&self) -> bool {
        !self.is_valid_at(Utc::now())
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn card(exp_month: i32, exp_year: i32) -> SavedCard {
        SavedCard {
            id: Uuid::nil(),
            member_id: Uuid::nil(),
            stripe_payment_method_id: String::new(),
            card_last_four: String::new(),
            card_brand: String::new(),
            exp_month,
            exp_year,
            is_default: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn day(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 12, 0, 0).unwrap()
    }

    #[test]
    fn valid_through_end_of_exp_month() {
        let c = card(5, 2026); // valid through May 2026
        assert!(c.is_valid_at(day(2026, 5, 1)), "first of exp month");
        assert!(c.is_valid_at(day(2026, 5, 31)), "last day of exp month");
        assert!(!c.is_valid_at(day(2026, 6, 1)), "first of next month");
    }

    #[test]
    fn before_expiry() {
        let c = card(5, 2026);
        assert!(c.is_valid_at(day(2025, 12, 1)));
        assert!(c.is_valid_at(day(2026, 1, 1)));
        assert!(c.is_valid_at(day(2026, 4, 30)));
    }

    #[test]
    fn after_expiry() {
        let c = card(5, 2026);
        assert!(!c.is_valid_at(day(2026, 6, 15)));
        assert!(!c.is_valid_at(day(2027, 1, 1)));
    }

    #[test]
    fn december_year_boundary() {
        let c = card(12, 2025);
        assert!(c.is_valid_at(day(2025, 12, 31)));
        assert!(!c.is_valid_at(day(2026, 1, 1)));
    }
}
