use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

use crate::{
    domain::{
        CreateMembershipTypeRequest, MembershipTypeConfig, UpdateMembershipTypeRequest,
        default_membership_types, slugify,
    },
    error::{AppError, Result},
};

#[derive(FromRow)]
struct MembershipTypeRow {
    id: String,
    name: String,
    slug: String,
    description: Option<String>,
    color: Option<String>,
    icon: Option<String>,
    sort_order: i32,
    is_active: i32,
    fee_cents: i32,
    billing_period: String,
    created_at: NaiveDateTime,
    updated_at: NaiveDateTime,
}

#[async_trait]
pub trait MembershipTypeRepository: Send + Sync {
    async fn create(&self, request: CreateMembershipTypeRequest) -> Result<MembershipTypeConfig>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<MembershipTypeConfig>>;
    async fn find_by_slug(&self, slug: &str) -> Result<Option<MembershipTypeConfig>>;
    async fn list(&self, include_inactive: bool) -> Result<Vec<MembershipTypeConfig>>;
    async fn update(&self, id: Uuid, request: UpdateMembershipTypeRequest) -> Result<MembershipTypeConfig>;
    async fn delete(&self, id: Uuid) -> Result<()>;
    async fn count_usage(&self, id: Uuid) -> Result<i64>;
    async fn get_next_sort_order(&self) -> Result<i32>;
    async fn reorder(&self, ids: &[Uuid]) -> Result<()>;
    async fn seed_defaults(&self) -> Result<Vec<MembershipTypeConfig>>;
}

pub struct SqliteMembershipTypeRepository {
    pool: SqlitePool,
}

impl SqliteMembershipTypeRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    fn row_to_config(row: MembershipTypeRow) -> Result<MembershipTypeConfig> {
        Ok(MembershipTypeConfig {
            id: Uuid::parse_str(&row.id).map_err(|e| AppError::Database(e.to_string()))?,
            name: row.name,
            slug: row.slug,
            description: row.description,
            color: row.color,
            icon: row.icon,
            sort_order: row.sort_order,
            is_active: row.is_active != 0,
            fee_cents: row.fee_cents,
            billing_period: row.billing_period,
            created_at: DateTime::from_naive_utc_and_offset(row.created_at, Utc),
            updated_at: DateTime::from_naive_utc_and_offset(row.updated_at, Utc),
        })
    }
}

#[async_trait]
impl MembershipTypeRepository for SqliteMembershipTypeRepository {
    async fn create(&self, request: CreateMembershipTypeRequest) -> Result<MembershipTypeConfig> {
        let id = Uuid::new_v4();
        let id_str = id.to_string();
        let slug = request.slug.unwrap_or_else(|| slugify(&request.name));
        let now = Utc::now().naive_utc();
        let sort_order = self.get_next_sort_order().await?;

        sqlx::query(
            r#"
            INSERT INTO membership_types (
                id, name, slug, description, color, icon,
                sort_order, is_active, fee_cents, billing_period,
                created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, 1, ?, ?, ?, ?)
            "#,
        )
        .bind(&id_str)
        .bind(&request.name)
        .bind(&slug)
        .bind(&request.description)
        .bind(&request.color)
        .bind(&request.icon)
        .bind(sort_order)
        .bind(request.fee_cents)
        .bind(&request.billing_period)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        self.find_by_id(id)
            .await?
            .ok_or_else(|| AppError::Database("Failed to retrieve created membership type".to_string()))
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<MembershipTypeConfig>> {
        let id_str = id.to_string();
        let row = sqlx::query_as::<_, MembershipTypeRow>(
            r#"
            SELECT id, name, slug, description, color, icon,
                   sort_order, is_active, fee_cents, billing_period,
                   created_at, updated_at
            FROM membership_types
            WHERE id = ?
            "#,
        )
        .bind(&id_str)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        match row {
            Some(r) => Ok(Some(Self::row_to_config(r)?)),
            None => Ok(None),
        }
    }

    async fn find_by_slug(&self, slug: &str) -> Result<Option<MembershipTypeConfig>> {
        let row = sqlx::query_as::<_, MembershipTypeRow>(
            r#"
            SELECT id, name, slug, description, color, icon,
                   sort_order, is_active, fee_cents, billing_period,
                   created_at, updated_at
            FROM membership_types
            WHERE slug = ?
            "#,
        )
        .bind(slug)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        match row {
            Some(r) => Ok(Some(Self::row_to_config(r)?)),
            None => Ok(None),
        }
    }

    async fn list(&self, include_inactive: bool) -> Result<Vec<MembershipTypeConfig>> {
        let query = if include_inactive {
            r#"
            SELECT id, name, slug, description, color, icon,
                   sort_order, is_active, fee_cents, billing_period,
                   created_at, updated_at
            FROM membership_types
            ORDER BY sort_order ASC, name ASC
            "#
        } else {
            r#"
            SELECT id, name, slug, description, color, icon,
                   sort_order, is_active, fee_cents, billing_period,
                   created_at, updated_at
            FROM membership_types
            WHERE is_active = 1
            ORDER BY sort_order ASC, name ASC
            "#
        };

        let rows = sqlx::query_as::<_, MembershipTypeRow>(query)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        rows.into_iter().map(Self::row_to_config).collect()
    }

    async fn update(&self, id: Uuid, request: UpdateMembershipTypeRequest) -> Result<MembershipTypeConfig> {
        let existing = self.find_by_id(id).await?.ok_or_else(|| {
            AppError::NotFound("Membership type not found".to_string())
        })?;

        let id_str = id.to_string();
        let now = Utc::now().naive_utc();

        let name = request.name.unwrap_or(existing.name);
        let description = request.description.or(existing.description);
        let color = request.color.or(existing.color);
        let icon = request.icon.or(existing.icon);
        let sort_order = request.sort_order.unwrap_or(existing.sort_order);
        let is_active = request.is_active.unwrap_or(existing.is_active);
        let fee_cents = request.fee_cents.unwrap_or(existing.fee_cents);
        let billing_period = request.billing_period.unwrap_or(existing.billing_period);

        sqlx::query(
            r#"
            UPDATE membership_types
            SET name = ?, description = ?, color = ?, icon = ?,
                sort_order = ?, is_active = ?, fee_cents = ?, billing_period = ?,
                updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(&name)
        .bind(&description)
        .bind(&color)
        .bind(&icon)
        .bind(sort_order)
        .bind(if is_active { 1i32 } else { 0i32 })
        .bind(fee_cents)
        .bind(&billing_period)
        .bind(now)
        .bind(&id_str)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        self.find_by_id(id)
            .await?
            .ok_or_else(|| AppError::Database("Failed to retrieve updated membership type".to_string()))
    }

    async fn delete(&self, id: Uuid) -> Result<()> {
        let id_str = id.to_string();

        sqlx::query("DELETE FROM membership_types WHERE id = ?")
            .bind(&id_str)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }

    async fn count_usage(&self, id: Uuid) -> Result<i64> {
        let id_str = id.to_string();

        let row: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*) as count
            FROM members
            WHERE membership_type_id = ?
            "#,
        )
        .bind(&id_str)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(row.0)
    }

    async fn get_next_sort_order(&self) -> Result<i32> {
        let row: (Option<i32>,) = sqlx::query_as(
            "SELECT MAX(sort_order) FROM membership_types"
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(row.0.unwrap_or(0) + 1)
    }

    async fn reorder(&self, ids: &[Uuid]) -> Result<()> {
        for (index, id) in ids.iter().enumerate() {
            let id_str = id.to_string();
            sqlx::query("UPDATE membership_types SET sort_order = ? WHERE id = ?")
                .bind(index as i32)
                .bind(&id_str)
                .execute(&self.pool)
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;
        }
        Ok(())
    }

    async fn seed_defaults(&self) -> Result<Vec<MembershipTypeConfig>> {
        let defaults = default_membership_types();
        let mut created = Vec::new();

        for (index, (name, slug, color, fee_cents, billing_period)) in defaults.into_iter().enumerate() {
            // Skip if already exists
            if self.find_by_slug(slug).await?.is_some() {
                continue;
            }

            let id = Uuid::new_v4();
            let id_str = id.to_string();
            let now = Utc::now().naive_utc();

            sqlx::query(
                r#"
                INSERT INTO membership_types (
                    id, name, slug, description, color, icon,
                    sort_order, is_active, fee_cents, billing_period,
                    created_at, updated_at
                ) VALUES (?, ?, ?, NULL, ?, NULL, ?, 1, ?, ?, ?, ?)
                "#,
            )
            .bind(&id_str)
            .bind(name)
            .bind(slug)
            .bind(color)
            .bind(index as i32)
            .bind(fee_cents)
            .bind(billing_period)
            .bind(now)
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

            if let Some(config) = self.find_by_id(id).await? {
                created.push(config);
            }
        }

        Ok(created)
    }
}
