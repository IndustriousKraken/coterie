//! Unified repository for event types and announcement types. The two kinds
//! are physically separate tables (`event_types`, `announcement_types`) with
//! identical column shapes; the `BasicTypeKind` discriminator threads through
//! every method so a single repository instance serves both kinds.
//!
//! SQL strings interpolate `kind.table()` / `kind.usage_table()` /
//! `kind.usage_fk()` via `format!`. Those values are compile-time `&'static
//! str` constants — never user input — so the interpolation is safe.

use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

use crate::{
    domain::{
        slugify, BasicType, BasicTypeKind, CreateBasicTypeRequest, UpdateBasicTypeRequest,
    },
    error::{AppError, Result},
};

#[derive(FromRow)]
struct BasicTypeRow {
    id: String,
    name: String,
    slug: String,
    description: Option<String>,
    color: Option<String>,
    icon: Option<String>,
    sort_order: i32,
    is_active: i32,
    created_at: NaiveDateTime,
    updated_at: NaiveDateTime,
}

#[async_trait]
pub trait BasicTypeRepository: Send + Sync {
    async fn create(
        &self,
        kind: BasicTypeKind,
        request: CreateBasicTypeRequest,
    ) -> Result<BasicType>;
    async fn find_by_id(&self, kind: BasicTypeKind, id: Uuid) -> Result<Option<BasicType>>;
    async fn find_by_slug(&self, kind: BasicTypeKind, slug: &str) -> Result<Option<BasicType>>;
    async fn list(&self, kind: BasicTypeKind, include_inactive: bool) -> Result<Vec<BasicType>>;
    async fn update(
        &self,
        kind: BasicTypeKind,
        id: Uuid,
        request: UpdateBasicTypeRequest,
    ) -> Result<BasicType>;
    async fn delete(&self, kind: BasicTypeKind, id: Uuid) -> Result<()>;
    async fn count_usage(&self, kind: BasicTypeKind, id: Uuid) -> Result<i64>;
    async fn get_next_sort_order(&self, kind: BasicTypeKind) -> Result<i32>;
}

pub struct SqliteBasicTypeRepository {
    pool: SqlitePool,
}

impl SqliteBasicTypeRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    fn row_to_config(row: BasicTypeRow) -> Result<BasicType> {
        Ok(BasicType {
            id: Uuid::parse_str(&row.id).map_err(|e| AppError::Internal(e.to_string()))?,
            name: row.name,
            slug: row.slug,
            description: row.description,
            color: row.color,
            icon: row.icon,
            sort_order: row.sort_order,
            is_active: row.is_active != 0,
            created_at: DateTime::from_naive_utc_and_offset(row.created_at, Utc),
            updated_at: DateTime::from_naive_utc_and_offset(row.updated_at, Utc),
        })
    }
}

#[async_trait]
impl BasicTypeRepository for SqliteBasicTypeRepository {
    async fn create(
        &self,
        kind: BasicTypeKind,
        request: CreateBasicTypeRequest,
    ) -> Result<BasicType> {
        let id = Uuid::new_v4();
        let id_str = id.to_string();
        let slug = request.slug.unwrap_or_else(|| slugify(&request.name));
        let now = Utc::now().naive_utc();
        let sort_order = self.get_next_sort_order(kind).await?;

        let sql = format!(
            "INSERT INTO {} (\
                id, name, slug, description, color, icon, \
                sort_order, is_active, created_at, updated_at\
             ) VALUES (?, ?, ?, ?, ?, ?, ?, 1, ?, ?)",
            kind.table()
        );

        sqlx::query(&sql)
            .bind(&id_str)
            .bind(&request.name)
            .bind(&slug)
            .bind(&request.description)
            .bind(&request.color)
            .bind(&request.icon)
            .bind(sort_order)
            .bind(now)
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;

        self.find_by_id(kind, id).await?.ok_or_else(|| {
            AppError::Internal(format!(
                "Failed to retrieve created {}",
                kind.display_name()
            ))
        })
    }

    async fn find_by_id(&self, kind: BasicTypeKind, id: Uuid) -> Result<Option<BasicType>> {
        let id_str = id.to_string();
        let sql = format!(
            "SELECT id, name, slug, description, color, icon, \
                    sort_order, is_active, created_at, updated_at \
             FROM {} \
             WHERE id = ?",
            kind.table()
        );
        let row = sqlx::query_as::<_, BasicTypeRow>(&sql)
            .bind(&id_str)
            .fetch_optional(&self.pool)
            .await
            .map_err(AppError::Database)?;

        match row {
            Some(r) => Ok(Some(Self::row_to_config(r)?)),
            None => Ok(None),
        }
    }

    async fn find_by_slug(
        &self,
        kind: BasicTypeKind,
        slug: &str,
    ) -> Result<Option<BasicType>> {
        let sql = format!(
            "SELECT id, name, slug, description, color, icon, \
                    sort_order, is_active, created_at, updated_at \
             FROM {} \
             WHERE slug = ?",
            kind.table()
        );
        let row = sqlx::query_as::<_, BasicTypeRow>(&sql)
            .bind(slug)
            .fetch_optional(&self.pool)
            .await
            .map_err(AppError::Database)?;

        match row {
            Some(r) => Ok(Some(Self::row_to_config(r)?)),
            None => Ok(None),
        }
    }

    async fn list(
        &self,
        kind: BasicTypeKind,
        include_inactive: bool,
    ) -> Result<Vec<BasicType>> {
        let sql = if include_inactive {
            format!(
                "SELECT id, name, slug, description, color, icon, \
                        sort_order, is_active, created_at, updated_at \
                 FROM {} \
                 ORDER BY sort_order ASC, name ASC",
                kind.table()
            )
        } else {
            format!(
                "SELECT id, name, slug, description, color, icon, \
                        sort_order, is_active, created_at, updated_at \
                 FROM {} \
                 WHERE is_active = 1 \
                 ORDER BY sort_order ASC, name ASC",
                kind.table()
            )
        };

        let rows = sqlx::query_as::<_, BasicTypeRow>(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(AppError::Database)?;

        rows.into_iter().map(Self::row_to_config).collect()
    }

    async fn update(
        &self,
        kind: BasicTypeKind,
        id: Uuid,
        request: UpdateBasicTypeRequest,
    ) -> Result<BasicType> {
        let existing = self.find_by_id(kind, id).await?.ok_or_else(|| {
            AppError::NotFound(format!(
                "{} not found",
                capitalize_first(kind.display_name())
            ))
        })?;

        let id_str = id.to_string();
        let now = Utc::now().naive_utc();

        let name = request.name.unwrap_or(existing.name);
        let description = request.description.or(existing.description);
        let color = request.color.or(existing.color);
        let icon = request.icon.or(existing.icon);
        let sort_order = request.sort_order.unwrap_or(existing.sort_order);
        let is_active = request.is_active.unwrap_or(existing.is_active);

        let sql = format!(
            "UPDATE {} \
             SET name = ?, description = ?, color = ?, icon = ?, \
                 sort_order = ?, is_active = ?, updated_at = ? \
             WHERE id = ?",
            kind.table()
        );

        sqlx::query(&sql)
            .bind(&name)
            .bind(&description)
            .bind(&color)
            .bind(&icon)
            .bind(sort_order)
            .bind(if is_active { 1i32 } else { 0i32 })
            .bind(now)
            .bind(&id_str)
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;

        self.find_by_id(kind, id).await?.ok_or_else(|| {
            AppError::Internal(format!(
                "Failed to retrieve updated {}",
                kind.display_name()
            ))
        })
    }

    async fn delete(&self, kind: BasicTypeKind, id: Uuid) -> Result<()> {
        let id_str = id.to_string();
        let sql = format!("DELETE FROM {} WHERE id = ?", kind.table());

        sqlx::query(&sql)
            .bind(&id_str)
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;

        Ok(())
    }

    async fn count_usage(&self, kind: BasicTypeKind, id: Uuid) -> Result<i64> {
        let id_str = id.to_string();
        let sql = format!(
            "SELECT COUNT(*) as count FROM {} WHERE {} = ?",
            kind.usage_table(),
            kind.usage_fk()
        );

        let row: (i64,) = sqlx::query_as(&sql)
            .bind(&id_str)
            .fetch_one(&self.pool)
            .await
            .map_err(AppError::Database)?;

        Ok(row.0)
    }

    async fn get_next_sort_order(&self, kind: BasicTypeKind) -> Result<i32> {
        let sql = format!("SELECT MAX(sort_order) FROM {}", kind.table());
        let row: (Option<i32>,) = sqlx::query_as(&sql)
            .fetch_one(&self.pool)
            .await
            .map_err(AppError::Database)?;

        Ok(row.0.unwrap_or(0) + 1)
    }
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}
