//! Repository for the `expenses` ledger table.
//!
//! Mutations are single-statement and inherit SQLite's row-level
//! locking, so callers SHOULD treat them as idempotent at the SQL
//! layer — a concurrent create with a duplicate id is impossible
//! because the service mints fresh UUIDs. Aggregate methods
//! (`sum_by_account`, `sum_by_category`) are pure reads.

use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::{FromRow, QueryBuilder, Sqlite, SqlitePool};
use uuid::Uuid;

use crate::{
    domain::{CreateExpenseRequest, Expense, UpdateExpenseRequest},
    error::{AppError, Result},
};

/// Inclusive-start, exclusive-end date range used to scope list and
/// aggregate queries. `start <= spent_at < end`.
#[derive(Debug, Clone, Copy)]
pub struct DateRange {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

#[derive(Debug, Clone, Default)]
pub struct ExpenseFilter {
    pub date_range: Option<DateRange>,
    pub category_id: Option<Uuid>,
    pub account_id: Option<Uuid>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ExpenseSum {
    pub key_id: Uuid,
    pub total_cents: i64,
}

#[async_trait]
pub trait ExpenseRepository: Send + Sync {
    async fn create(&self, created_by: Uuid, request: CreateExpenseRequest) -> Result<Expense>;
    async fn update(&self, id: Uuid, request: UpdateExpenseRequest) -> Result<Expense>;
    async fn delete(&self, id: Uuid) -> Result<()>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<Expense>>;
    async fn list(&self, filter: ExpenseFilter) -> Result<Vec<Expense>>;
    /// Total number of expense rows matching the same filter the
    /// caller passed to `list` (used for paginated UIs that need a
    /// row count alongside the page slice). Ignores `limit` / `offset`.
    async fn count(&self, filter: ExpenseFilter) -> Result<i64>;
    async fn sum_by_account(&self, range: DateRange) -> Result<Vec<ExpenseSum>>;
    async fn sum_by_category(&self, range: DateRange) -> Result<Vec<ExpenseSum>>;
    /// Sum of all expense cents in the range — used by the monthly
    /// reconciliation report's "net" line.
    async fn total_in_range(&self, range: DateRange) -> Result<i64>;
}

#[derive(FromRow)]
struct ExpenseRow {
    id: String,
    spent_at: NaiveDateTime,
    amount_cents: i64,
    currency: String,
    description: String,
    category_id: String,
    account_id: String,
    notes: Option<String>,
    created_by: String,
    created_at: NaiveDateTime,
    updated_at: NaiveDateTime,
}

pub struct SqliteExpenseRepository {
    pool: SqlitePool,
}

impl SqliteExpenseRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    fn row_to_expense(row: ExpenseRow) -> Result<Expense> {
        Ok(Expense {
            id: Uuid::parse_str(&row.id).map_err(|e| AppError::Internal(e.to_string()))?,
            spent_at: DateTime::from_naive_utc_and_offset(row.spent_at, Utc),
            amount_cents: row.amount_cents,
            currency: row.currency,
            description: row.description,
            category_id: Uuid::parse_str(&row.category_id)
                .map_err(|e| AppError::Internal(e.to_string()))?,
            account_id: Uuid::parse_str(&row.account_id)
                .map_err(|e| AppError::Internal(e.to_string()))?,
            notes: row.notes,
            created_by: Uuid::parse_str(&row.created_by)
                .map_err(|e| AppError::Internal(e.to_string()))?,
            created_at: DateTime::from_naive_utc_and_offset(row.created_at, Utc),
            updated_at: DateTime::from_naive_utc_and_offset(row.updated_at, Utc),
        })
    }

    fn apply_filter_where<'a>(builder: &mut QueryBuilder<'a, Sqlite>, filter: &'a ExpenseFilter) {
        let mut first = true;
        let mut push_clause = |b: &mut QueryBuilder<'a, Sqlite>| {
            if first {
                b.push(" WHERE ");
                first = false;
            } else {
                b.push(" AND ");
            }
        };

        if let Some(range) = filter.date_range {
            push_clause(builder);
            builder.push("spent_at >= ");
            builder.push_bind(range.start.naive_utc());
            builder.push(" AND spent_at < ");
            builder.push_bind(range.end.naive_utc());
        }
        if let Some(cid) = filter.category_id {
            push_clause(builder);
            builder.push("category_id = ");
            builder.push_bind(cid.to_string());
        }
        if let Some(aid) = filter.account_id {
            push_clause(builder);
            builder.push("account_id = ");
            builder.push_bind(aid.to_string());
        }
    }
}

#[async_trait]
impl ExpenseRepository for SqliteExpenseRepository {
    async fn create(&self, created_by: Uuid, request: CreateExpenseRequest) -> Result<Expense> {
        let id = Uuid::new_v4();
        let now = Utc::now().naive_utc();
        let currency = request
            .currency
            .clone()
            .unwrap_or_else(|| "USD".to_string());

        sqlx::query(
            "INSERT INTO expenses ( \
                id, spent_at, amount_cents, currency, description, \
                category_id, account_id, notes, created_by, \
                created_at, updated_at \
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(request.spent_at.naive_utc())
        .bind(request.amount_cents)
        .bind(&currency)
        .bind(&request.description)
        .bind(request.category_id.to_string())
        .bind(request.account_id.to_string())
        .bind(&request.notes)
        .bind(created_by.to_string())
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;

        self.find_by_id(id)
            .await?
            .ok_or_else(|| AppError::Internal("Failed to retrieve created expense".into()))
    }

    async fn update(&self, id: Uuid, request: UpdateExpenseRequest) -> Result<Expense> {
        let existing = self
            .find_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound("Expense not found".into()))?;

        let spent_at = request.spent_at.unwrap_or(existing.spent_at);
        let amount_cents = request.amount_cents.unwrap_or(existing.amount_cents);
        let currency = request.currency.unwrap_or(existing.currency);
        let description = request.description.unwrap_or(existing.description);
        let category_id = request.category_id.unwrap_or(existing.category_id);
        let account_id = request.account_id.unwrap_or(existing.account_id);
        // Notes uses Option with .or so callers can clear the field via
        // an explicit empty-string POST handled at the service layer.
        let notes = request.notes.or(existing.notes);
        let now = Utc::now().naive_utc();

        sqlx::query(
            "UPDATE expenses \
             SET spent_at = ?, amount_cents = ?, currency = ?, description = ?, \
                 category_id = ?, account_id = ?, notes = ?, updated_at = ? \
             WHERE id = ?",
        )
        .bind(spent_at.naive_utc())
        .bind(amount_cents)
        .bind(&currency)
        .bind(&description)
        .bind(category_id.to_string())
        .bind(account_id.to_string())
        .bind(&notes)
        .bind(now)
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;

        self.find_by_id(id)
            .await?
            .ok_or_else(|| AppError::Internal("Failed to retrieve updated expense".into()))
    }

    async fn delete(&self, id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM expenses WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;
        Ok(())
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<Expense>> {
        let row = sqlx::query_as::<_, ExpenseRow>(
            "SELECT id, spent_at, amount_cents, currency, description, \
                    category_id, account_id, notes, created_by, \
                    created_at, updated_at \
             FROM expenses WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        match row {
            Some(r) => Ok(Some(Self::row_to_expense(r)?)),
            None => Ok(None),
        }
    }

    async fn list(&self, filter: ExpenseFilter) -> Result<Vec<Expense>> {
        let mut builder: QueryBuilder<Sqlite> = QueryBuilder::new(
            "SELECT id, spent_at, amount_cents, currency, description, \
                    category_id, account_id, notes, created_by, \
                    created_at, updated_at \
             FROM expenses",
        );
        Self::apply_filter_where(&mut builder, &filter);
        builder.push(" ORDER BY spent_at DESC, created_at DESC");
        if let Some(limit) = filter.limit {
            builder.push(" LIMIT ");
            builder.push_bind(limit);
        }
        if let Some(offset) = filter.offset {
            builder.push(" OFFSET ");
            builder.push_bind(offset);
        }

        let rows = builder
            .build_query_as::<ExpenseRow>()
            .fetch_all(&self.pool)
            .await
            .map_err(AppError::Database)?;
        rows.into_iter().map(Self::row_to_expense).collect()
    }

    async fn count(&self, filter: ExpenseFilter) -> Result<i64> {
        let mut builder: QueryBuilder<Sqlite> = QueryBuilder::new("SELECT COUNT(*) FROM expenses");
        Self::apply_filter_where(&mut builder, &filter);
        let (count,): (i64,) = builder
            .build_query_as()
            .fetch_one(&self.pool)
            .await
            .map_err(AppError::Database)?;
        Ok(count)
    }

    async fn sum_by_account(&self, range: DateRange) -> Result<Vec<ExpenseSum>> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT account_id, SUM(amount_cents) AS total \
             FROM expenses \
             WHERE spent_at >= ? AND spent_at < ? \
             GROUP BY account_id",
        )
        .bind(range.start.naive_utc())
        .bind(range.end.naive_utc())
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        rows.into_iter()
            .map(|(id, total)| {
                Ok(ExpenseSum {
                    key_id: Uuid::parse_str(&id).map_err(|e| AppError::Internal(e.to_string()))?,
                    total_cents: total,
                })
            })
            .collect()
    }

    async fn sum_by_category(&self, range: DateRange) -> Result<Vec<ExpenseSum>> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT category_id, SUM(amount_cents) AS total \
             FROM expenses \
             WHERE spent_at >= ? AND spent_at < ? \
             GROUP BY category_id",
        )
        .bind(range.start.naive_utc())
        .bind(range.end.naive_utc())
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        rows.into_iter()
            .map(|(id, total)| {
                Ok(ExpenseSum {
                    key_id: Uuid::parse_str(&id).map_err(|e| AppError::Internal(e.to_string()))?,
                    total_cents: total,
                })
            })
            .collect()
    }

    async fn total_in_range(&self, range: DateRange) -> Result<i64> {
        let total: Option<i64> = sqlx::query_scalar(
            "SELECT COALESCE(SUM(amount_cents), 0) FROM expenses \
             WHERE spent_at >= ? AND spent_at < ?",
        )
        .bind(range.start.naive_utc())
        .bind(range.end.naive_utc())
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(total.unwrap_or(0))
    }
}
