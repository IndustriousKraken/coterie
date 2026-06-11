//! Repository for the `expense_accounts` lookup table — named payment
//! instruments operators attach to expenses.
//!
//! Each method maps to a single SQL statement and inherits SQLite's
//! row-level locking. Callers SHOULD treat mutating methods as
//! idempotent at the SQL layer (a duplicate name surfaces as a
//! Conflict, not silent insert).

use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

use crate::{
    domain::{CreateExpenseAccountRequest, ExpenseAccount, UpdateExpenseAccountRequest},
    error::{AppError, Result},
};

#[async_trait]
pub trait ExpenseAccountRepository: Send + Sync {
    async fn create(&self, request: CreateExpenseAccountRequest) -> Result<ExpenseAccount>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<ExpenseAccount>>;
    async fn list(&self, include_inactive: bool) -> Result<Vec<ExpenseAccount>>;
    async fn update(
        &self,
        id: Uuid,
        request: UpdateExpenseAccountRequest,
    ) -> Result<ExpenseAccount>;
    async fn delete(&self, id: Uuid) -> Result<()>;
    /// Returns the number of `expenses` rows referencing this account.
    /// Used by the service layer to refuse hard-delete in favour of
    /// deactivation (`is_active = 0`) when the account is in use.
    async fn count_referencing_expenses(&self, account_id: Uuid) -> Result<i64>;
}

#[derive(FromRow)]
struct AccountRow {
    id: String,
    name: String,
    is_active: i64,
    sort_order: i64,
    created_at: NaiveDateTime,
}

pub struct SqliteExpenseAccountRepository {
    pool: SqlitePool,
}

impl SqliteExpenseAccountRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    fn row_to_account(row: AccountRow) -> Result<ExpenseAccount> {
        Ok(ExpenseAccount {
            id: Uuid::parse_str(&row.id).map_err(|e| AppError::Internal(e.to_string()))?,
            name: row.name,
            is_active: row.is_active != 0,
            sort_order: row.sort_order as i32,
            created_at: DateTime::from_naive_utc_and_offset(row.created_at, Utc),
        })
    }

    async fn next_sort_order(&self) -> Result<i32> {
        let row: (Option<i64>,) = sqlx::query_as("SELECT MAX(sort_order) FROM expense_accounts")
            .fetch_one(&self.pool)
            .await
            .map_err(AppError::Database)?;
        Ok(row.0.unwrap_or(0) as i32 + 1)
    }
}

#[async_trait]
impl ExpenseAccountRepository for SqliteExpenseAccountRepository {
    async fn create(&self, request: CreateExpenseAccountRequest) -> Result<ExpenseAccount> {
        let id = Uuid::new_v4();
        let sort = self.next_sort_order().await?;

        sqlx::query(
            "INSERT INTO expense_accounts (id, name, is_active, sort_order) \
             VALUES (?, ?, 1, ?)",
        )
        .bind(id.to_string())
        .bind(&request.name)
        .bind(sort as i64)
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;

        self.find_by_id(id)
            .await?
            .ok_or_else(|| AppError::Internal("Failed to retrieve created expense account".into()))
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<ExpenseAccount>> {
        let row = sqlx::query_as::<_, AccountRow>(
            "SELECT id, name, is_active, sort_order, created_at \
             FROM expense_accounts WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        match row {
            Some(r) => Ok(Some(Self::row_to_account(r)?)),
            None => Ok(None),
        }
    }

    async fn list(&self, include_inactive: bool) -> Result<Vec<ExpenseAccount>> {
        let sql = if include_inactive {
            "SELECT id, name, is_active, sort_order, created_at \
             FROM expense_accounts \
             ORDER BY sort_order ASC, name ASC"
        } else {
            "SELECT id, name, is_active, sort_order, created_at \
             FROM expense_accounts \
             WHERE is_active = 1 \
             ORDER BY sort_order ASC, name ASC"
        };

        let rows = sqlx::query_as::<_, AccountRow>(sql)
            .fetch_all(&self.pool)
            .await
            .map_err(AppError::Database)?;
        rows.into_iter().map(Self::row_to_account).collect()
    }

    async fn update(
        &self,
        id: Uuid,
        request: UpdateExpenseAccountRequest,
    ) -> Result<ExpenseAccount> {
        let existing = self
            .find_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound("Expense account not found".into()))?;

        let name = request.name.unwrap_or(existing.name);
        let sort_order = request.sort_order.unwrap_or(existing.sort_order);
        let is_active = request.is_active.unwrap_or(existing.is_active);

        sqlx::query(
            "UPDATE expense_accounts \
             SET name = ?, sort_order = ?, is_active = ? \
             WHERE id = ?",
        )
        .bind(&name)
        .bind(sort_order as i64)
        .bind(if is_active { 1i64 } else { 0i64 })
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;

        self.find_by_id(id)
            .await?
            .ok_or_else(|| AppError::Internal("Failed to retrieve updated expense account".into()))
    }

    async fn delete(&self, id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM expense_accounts WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;
        Ok(())
    }

    async fn count_referencing_expenses(&self, account_id: Uuid) -> Result<i64> {
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM expenses WHERE account_id = ?")
            .bind(account_id.to_string())
            .fetch_one(&self.pool)
            .await
            .map_err(AppError::Database)?;
        Ok(count)
    }
}
