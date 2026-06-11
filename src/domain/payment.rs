use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Hard ceiling on a single payment / donation / refund, in cents.
/// Picked to be well above any legitimate Coterie transaction
/// ($100k) but low enough that an unintended extra zero or a
/// scripted abuse attempt fails fast at the boundary instead of
/// hitting Stripe with a bogus amount.
pub const MAX_PAYMENT_CENTS: i64 = 10_000_000;

/// Who paid. Sum-type makes member-vs-public-donor mutually
/// exclusive — you can't construct a `Payment` with both a
/// `member_id` and a separate donor identity, which the previous
/// flat layout allowed and runtime checks had to police.
///
/// The DB still stores `member_id`, `donor_name`, `donor_email`
/// as separate nullable columns (with a CHECK constraint enforcing
/// "exactly one path is populated"). The repository's row→Payment
/// mapper validates the row and constructs the correct variant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Payer {
    /// Existing Coterie member paying through any flow (Checkout,
    /// saved card, manual entry, donation tied to their account).
    Member(Uuid),
    /// Anonymous donor coming through `POST /public/donate` whose
    /// email didn't match any existing member. We capture identity
    /// on the row so we can issue receipts and contact them later.
    PublicDonor { name: String, email: String },
}

impl Payer {
    /// Convenience: the member uuid if this is a member payment.
    /// Returns `None` for public-donor payments. Use this when you
    /// need "member-or-nothing" semantics; otherwise prefer a full
    /// `match` so future variants force you to handle them.
    pub fn member_id(&self) -> Option<Uuid> {
        match self {
            Payer::Member(id) => Some(*id),
            Payer::PublicDonor { .. } => None,
        }
    }
}

/// What this payment is for, with kind-specific data living on the
/// variant that uses it. Replaces the previous (`payment_type`,
/// `donation_campaign_id`) pair where the campaign field was only
/// meaningful for the Donation variant but the type didn't say so.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PaymentKind {
    /// Member dues. Triggers the dues-extension flow on completion.
    Membership,
    /// Charitable donation. `campaign_id` is the donation_campaigns
    /// row this counts toward, or `None` for a general donation.
    Donation { campaign_id: Option<Uuid> },
    /// Free-form bucket — merch, event fees, anything that's neither
    /// dues nor a donation. No automatic dues / campaign side-effects.
    Other,
}

impl PaymentKind {
    /// String form used in the `payment_type` DB column. Match the
    /// schema default ('membership') so older rows deserialize.
    pub fn as_str(&self) -> &'static str {
        match self {
            PaymentKind::Membership => "membership",
            PaymentKind::Donation { .. } => "donation",
            PaymentKind::Other => "other",
        }
    }

    /// The campaign id, if this is a `Donation`. Convenience for code
    /// that doesn't care about kind otherwise.
    pub fn campaign_id(&self) -> Option<Uuid> {
        match self {
            PaymentKind::Donation { campaign_id } => *campaign_id,
            _ => None,
        }
    }
}

/// External Stripe identifier we stored against the payment row.
/// Stripe's API takes different shapes depending on what flow created
/// the charge:
///
///   - `pi_…`  PaymentIntent (saved-card charges, subscription
///             payments processed by Coterie's billing runner)
///   - `cs_…`  CheckoutSession (one-time membership/donation
///             checkouts)
///   - `in_…`  Invoice (Stripe-managed subscription invoices)
///
/// Replaces the previous opaque `Option<String>` whose prefix was
/// dispatched on with stringly-typed `starts_with` checks. With the
/// enum, refund / lookup logic becomes a `match` the compiler proves
/// exhaustive; new variants force every site to handle them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum StripeRef {
    PaymentIntent(String),
    CheckoutSession(String),
    Invoice(String),
}

impl StripeRef {
    /// Parse from the raw Stripe id string. Returns `None` if the
    /// prefix doesn't match a known shape — caller should treat
    /// that as "not one of ours" rather than constructing a default.
    pub fn from_id(id: &str) -> Option<Self> {
        if id.starts_with("pi_") {
            Some(StripeRef::PaymentIntent(id.to_string()))
        } else if id.starts_with("cs_") {
            Some(StripeRef::CheckoutSession(id.to_string()))
        } else if id.starts_with("in_") {
            Some(StripeRef::Invoice(id.to_string()))
        } else {
            None
        }
    }

    /// The raw Stripe id string, prefix included. What we send back
    /// to Stripe (refund, retrieve) and what we store in the DB.
    pub fn as_str(&self) -> &str {
        match self {
            StripeRef::PaymentIntent(s) => s,
            StripeRef::CheckoutSession(s) => s,
            StripeRef::Invoice(s) => s,
        }
    }
}

/// A payment on file. Row-level facts (id, amount, currency, dates,
/// description) live as flat fields; the *who*, *what*, and
/// *external-system reference* live as sum types so illegal
/// combinations are unrepresentable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Payment {
    pub id: Uuid,
    /// Who paid — either a member or a public donor. See [`Payer`].
    pub payer: Payer,
    pub amount_cents: i64,
    pub currency: String,
    pub status: PaymentStatus,
    pub payment_method: PaymentMethod,
    /// What the payment is for — membership dues, a donation (with
    /// optional campaign), or something else. See [`PaymentKind`].
    pub kind: PaymentKind,
    /// Stripe-side reference if this payment touched Stripe. `None`
    /// for Manual / Waived rows. See [`StripeRef`].
    pub external_id: Option<StripeRef>,
    pub description: String,
    pub paid_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Payment {
    /// Convenience accessor mirroring the prior `member_id` field —
    /// returns the uuid for member payments, `None` for public donor
    /// payments. Prefer matching on `payer` directly when you need to
    /// branch; this exists for read-only equality checks like "does
    /// this payment belong to user X."
    pub fn member_id(&self) -> Option<Uuid> {
        self.payer.member_id()
    }
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
