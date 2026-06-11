//! Repository for the `expense_categories` lookup table — operator-
//! defined taxonomy attached to expense rows. Flat list; no
//! hierarchy in v1.
//!
//! Same idempotency expectations as `ExpenseAccountRepository`: each
//! method is a single SQL statement under SQLite's row-level locking;
//! duplicate names or slugs surface as a `Conflict` rather than a
//! silent insert.

use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

use crate::{
    domain::{
        slugify, CreateExpenseCategoryRequest, ExpenseCategory, UpdateExpenseCategoryRequest,
    },
    error::{AppError, Result},
};

#[async_trait]
pub trait ExpenseCategoryRepository: Send + Sync {
    async fn create(&self, request: CreateExpenseCategoryRequest) -> Result<ExpenseCategory>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<ExpenseCategory>>;
    async fn find_by_slug(&self, slug: &str) -> Result<Option<ExpenseCategory>>;
    async fn list(&self, include_inactive: bool) -> Result<Vec<ExpenseCategory>>;
    async fn update(
        &self,
        id: Uuid,
        request: UpdateExpenseCategoryRequest,
    ) -> Result<ExpenseCategory>;
    async fn delete(&self, id: Uuid) -> Result<()>;
    /// Returns the number of `expenses` rows referencing this category.
    /// Used by the service layer's soft-delete check.
    async fn count_referencing_expenses(&self, category_id: Uuid) -> Result<i64>;
}

#[derive(FromRow)]
struct CategoryRow {
    id: String,
    name: String,
    slug: String,
    is_active: i64,
    sort_order: i64,
    created_at: NaiveDateTime,
}

pub struct SqliteExpenseCategoryRepository {
    pool: SqlitePool,
}

impl SqliteExpenseCategoryRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    fn row_to_category(row: CategoryRow) -> Result<ExpenseCategory> {
        Ok(ExpenseCategory {
            id: Uuid::parse_str(&row.id).map_err(|e| AppError::Internal(e.to_string()))?,
            name: row.name,
            slug: row.slug,
            is_active: row.is_active != 0,
            sort_order: row.sort_order as i32,
            created_at: DateTime::from_naive_utc_and_offset(row.created_at, Utc),
        })
    }

    async fn next_sort_order(&self) -> Result<i32> {
        let row: (Option<i64>,) = sqlx::query_as("SELECT MAX(sort_order) FROM expense_categories")
            .fetch_one(&self.pool)
            .await
            .map_err(AppError::Database)?;
        Ok(row.0.unwrap_or(0) as i32 + 1)
    }
}

#[async_trait]
impl ExpenseCategoryRepository for SqliteExpenseCategoryRepository {
    async fn create(&self, request: CreateExpenseCategoryRequest) -> Result<ExpenseCategory> {
        let id = Uuid::new_v4();
        let slug = request.slug.unwrap_or_else(|| slugify(&request.name));
        let sort = self.next_sort_order().await?;

        sqlx::query(
            "INSERT INTO expense_categories (id, name, slug, is_active, sort_order) \
             VALUES (?, ?, ?, 1, ?)",
        )
        .bind(id.to_string())
        .bind(&request.name)
        .bind(&slug)
        .bind(sort as i64)
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;

        self.find_by_id(id)
            .await?
            .ok_or_else(|| AppError::Internal("Failed to retrieve created expense category".into()))
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<ExpenseCategory>> {
        let row = sqlx::query_as::<_, CategoryRow>(
            "SELECT id, name, slug, is_active, sort_order, created_at \
             FROM expense_categories WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        match row {
            Some(r) => Ok(Some(Self::row_to_category(r)?)),
            None => Ok(None),
        }
    }

    async fn find_by_slug(&self, slug: &str) -> Result<Option<ExpenseCategory>> {
        let row = sqlx::query_as::<_, CategoryRow>(
            "SELECT id, name, slug, is_active, sort_order, created_at \
             FROM expense_categories WHERE slug = ?",
        )
        .bind(slug)
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        match row {
            Some(r) => Ok(Some(Self::row_to_category(r)?)),
            None => Ok(None),
        }
    }

    async fn list(&self, include_inactive: bool) -> Result<Vec<ExpenseCategory>> {
        let sql = if include_inactive {
            "SELECT id, name, slug, is_active, sort_order, created_at \
             FROM expense_categories \
             ORDER BY sort_order ASC, name ASC"
        } else {
            "SELECT id, name, slug, is_active, sort_order, created_at \
             FROM expense_categories \
             WHERE is_active = 1 \
             ORDER BY sort_order ASC, name ASC"
        };

        let rows = sqlx::query_as::<_, CategoryRow>(sql)
            .fetch_all(&self.pool)
            .await
            .map_err(AppError::Database)?;
        rows.into_iter().map(Self::row_to_category).collect()
    }

    async fn update(
        &self,
        id: Uuid,
        request: UpdateExpenseCategoryRequest,
    ) -> Result<ExpenseCategory> {
        let existing = self
            .find_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound("Expense category not found".into()))?;

        let name = request.name.unwrap_or(existing.name);
        let sort_order = request.sort_order.unwrap_or(existing.sort_order);
        let is_active = request.is_active.unwrap_or(existing.is_active);

        sqlx::query(
            "UPDATE expense_categories \
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
            .ok_or_else(|| AppError::Internal("Failed to retrieve updated expense category".into()))
    }

    async fn delete(&self, id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM expense_categories WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;
        Ok(())
    }

    async fn count_referencing_expenses(&self, category_id: Uuid) -> Result<i64> {
        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM expenses WHERE category_id = ?")
                .bind(category_id.to_string())
                .fetch_one(&self.pool)
                .await
                .map_err(AppError::Database)?;
        Ok(count)
    }
}
