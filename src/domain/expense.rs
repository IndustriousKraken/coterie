use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Hard ceiling on a single expense, in cents. Same shape and
/// motivation as `MAX_PAYMENT_CENTS` — well above any legitimate
/// org outflow ($100k) but low enough that a stray extra zero fails
/// at the boundary instead of landing as a $1M typo in the ledger.
pub const MAX_EXPENSE_CENTS: i64 = 10_000_000;

/// A named payment instrument the org uses to pay for things — for
/// example "Debit Card 1 – Jane", "Debit Card 2 – Bob", "Petty
/// Cash". NOT an accounting account: no balance, no asset/liability
/// classification. Each expense picks exactly one.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ExpenseAccount {
    pub id: Uuid,
    pub name: String,
    pub is_active: bool,
    pub sort_order: i32,
    pub created_at: DateTime<Utc>,
}

/// Operator-defined expense taxonomy (Supplies, Software, Events,
/// Insurance, …). Flat list in v1 — no hierarchy. The slug is
/// stable across renames so reports can group by it.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ExpenseCategory {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub is_active: bool,
    pub sort_order: i32,
    pub created_at: DateTime<Utc>,
}

/// One outflow from the org — date on the receipt, amount, the
/// category and account attribution, and a free-form description.
/// Always non-negative: refunds for income live on the existing
/// `payments` table via the Stripe refund flow, not here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Expense {
    pub id: Uuid,
    pub spent_at: DateTime<Utc>,
    pub amount_cents: i64,
    pub currency: String,
    pub description: String,
    pub category_id: Uuid,
    pub account_id: Uuid,
    pub notes: Option<String>,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateExpenseRequest {
    pub spent_at: DateTime<Utc>,
    pub amount_cents: i64,
    pub currency: Option<String>,
    pub description: String,
    pub category_id: Uuid,
    pub account_id: Uuid,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateExpenseRequest {
    pub spent_at: Option<DateTime<Utc>>,
    pub amount_cents: Option<i64>,
    pub currency: Option<String>,
    pub description: Option<String>,
    pub category_id: Option<Uuid>,
    pub account_id: Option<Uuid>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateExpenseCategoryRequest {
    pub name: String,
    pub slug: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateExpenseCategoryRequest {
    pub name: Option<String>,
    pub sort_order: Option<i32>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateExpenseAccountRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateExpenseAccountRequest {
    pub name: Option<String>,
    pub sort_order: Option<i32>,
    pub is_active: Option<bool>,
}
