use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Hard ceiling on a single payment / donation / refund, in cents.
/// Picked to be well above any legitimate Coterie transaction
/// ($100k) but low enough that an unintended extra zero or a
/// scripted abuse attempt fails fast at the boundary instead of
/// hitting Stripe with a bogus amount.
pub const MAX_PAYMENT_CENTS: i64 = 10_000_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Payment {
    pub id: Uuid,
    pub member_id: Uuid,
    pub amount_cents: i64,
    pub currency: String,
    pub status: PaymentStatus,
    pub payment_method: PaymentMethod,
    pub stripe_payment_id: Option<String>,
    pub description: String,
    /// What kind of payment this is — drives whether dues get extended,
    /// where the row counts toward campaign totals, and so on.
    pub payment_type: PaymentType,
    /// FK to donation_campaigns. Set when payment_type=Donation; ignored
    /// otherwise. Used by `get_total_donated` for accurate progress.
    pub donation_campaign_id: Option<Uuid>,
    pub paid_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq)]
#[sqlx(type_name = "TEXT")]
pub enum PaymentStatus {
    Pending,
    Completed,
    Failed,
    Refunded,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq)]
#[sqlx(type_name = "TEXT")]
pub enum PaymentMethod {
    Stripe,
    Manual,
    Waived,
}

/// What this payment is for. Membership pays dues (extends
/// dues_paid_until); Donation feeds a campaign total (no dues
/// effect); Other is a free-form bucket for things like merch sales,
/// event fees, etc., that we don't want lumped with membership.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PaymentType {
    Membership,
    Donation,
    Other,
}

impl PaymentType {
    /// String form used in the DB column and Stripe metadata. Match the
    /// schema default ('membership') so existing rows deserialize.
    pub fn as_str(&self) -> &'static str {
        match self {
            PaymentType::Membership => "membership",
            PaymentType::Donation => "donation",
            PaymentType::Other => "other",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "membership" => Some(PaymentType::Membership),
            "donation" => Some(PaymentType::Donation),
            "other" => Some(PaymentType::Other),
            _ => None,
        }
    }
}

impl Default for PaymentType {
    fn default() -> Self {
        PaymentType::Membership
    }
}