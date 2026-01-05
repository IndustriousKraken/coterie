use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

use crate::{
    domain::{
        AnnouncementTypeConfig, CreateAnnouncementTypeRequest, UpdateAnnouncementTypeRequest,
        default_announcement_types, slugify,
    },
    error::{AppError, Result},
};

#[derive(FromRow)]
struct AnnouncementTypeRow {
    id: String,
    name: String,
    slug: String,
    description: Option<String>,
    color: Option<String>,
    icon: Option<String>,
    sort_order: i32,
    is_active: i32,
    is_system: i32,
    created_at: NaiveDateTime,
    updated_at: NaiveDateTime,
}

#[async_trait]
pub trait AnnouncementTypeRepository: Send + Sync {
    async fn create(&self, request: CreateAnnouncementTypeRequest) -> Result<AnnouncementTypeConfig>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<AnnouncementTypeConfig>>;
    async fn find_by_slug(&self, slug: &str) -> Result<Option<AnnouncementTypeConfig>>;
    async fn list(&self, include_inactive: bool) -> Result<Vec<AnnouncementTypeConfig>>;
    async fn update(&self, id: Uuid, request: UpdateAnnouncementTypeRequest) -> Result<AnnouncementTypeConfig>;
    async fn delete(&self, id: Uuid) -> Result<()>;
    async fn count_usage(&self, id: Uuid) -> Result<i64>;
    async fn get_next_sort_order(&self) -> Result<i32>;
    async fn reorder(&self, ids: &[Uuid]) -> Result<()>;
    async fn seed_defaults(&self) -> Result<Vec<AnnouncementTypeConfig>>;
}

pub struct SqliteAnnouncementTypeRepository {
    pool: SqlitePool,
}

impl SqliteAnnouncementTypeRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    fn row_to_config(row: AnnouncementTypeRow) -> Result<AnnouncementTypeConfig> {
        Ok(AnnouncementTypeConfig {
            id: Uuid::parse_str(&row.id).map_err(|e| AppError::Database(e.to_string()))?,
            name: row.name,
            slug: row.slug,
            description: row.description,
            color: row.color,
            icon: row.icon,
            sort_order: row.sort_order,
            is_active: row.is_active != 0,
            is_system: row.is_system != 0,
            created_at: DateTime::from_naive_utc_and_offset(row.created_at, Utc),
            updated_at: DateTime::from_naive_utc_and_offset(row.updated_at, Utc),
        })
    }
}

#[async_trait]
impl AnnouncementTypeRepository for SqliteAnnouncementTypeRepository {
    async fn create(&self, request: CreateAnnouncementTypeRequest) -> Result<AnnouncementTypeConfig> {
        let id = Uuid::new_v4();
        let id_str = id.to_string();
        let slug = request.slug.unwrap_or_else(|| slugify(&request.name));
        let now = Utc::now().naive_utc();
        let sort_order = self.get_next_sort_order().await?;

        sqlx::query(
            r#"
            INSERT INTO announcement_types (
                id, name, slug, description, color, icon,
                sort_order, is_active, is_system, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, 1, 0, ?, ?)
            "#,
        )
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
        .map_err(|e| AppError::Database(e.to_string()))?;

        self.find_by_id(id)
            .await?
            .ok_or_else(|| AppError::Database("Failed to retrieve created announcement type".to_string()))
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<AnnouncementTypeConfig>> {
        let id_str = id.to_string();
        let row = sqlx::query_as::<_, AnnouncementTypeRow>(
            r#"
            SELECT id, name, slug, description, color, icon,
                   sort_order, is_active, is_system, created_at, updated_at
            FROM announcement_types
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

    async fn find_by_slug(&self, slug: &str) -> Result<Option<AnnouncementTypeConfig>> {
        let row = sqlx::query_as::<_, AnnouncementTypeRow>(
            r#"
            SELECT id, name, slug, description, color, icon,
                   sort_order, is_active, is_system, created_at, updated_at
            FROM announcement_types
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

    async fn list(&self, include_inactive: bool) -> Result<Vec<AnnouncementTypeConfig>> {
        let query = if include_inactive {
            r#"
            SELECT id, name, slug, description, color, icon,
                   sort_order, is_active, is_system, created_at, updated_at
            FROM announcement_types
            ORDER BY sort_order ASC, name ASC
            "#
        } else {
            r#"
            SELECT id, name, slug, description, color, icon,
                   sort_order, is_active, is_system, created_at, updated_at
            FROM announcement_types
            WHERE is_active = 1
            ORDER BY sort_order ASC, name ASC
            "#
        };

        let rows = sqlx::query_as::<_, AnnouncementTypeRow>(query)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        rows.into_iter().map(Self::row_to_config).collect()
    }

    async fn update(&self, id: Uuid, request: UpdateAnnouncementTypeRequest) -> Result<AnnouncementTypeConfig> {
        let existing = self.find_by_id(id).await?.ok_or_else(|| {
            AppError::NotFound("Announcement type not found".to_string())
        })?;

        let id_str = id.to_string();
        let now = Utc::now().naive_utc();

        let name = request.name.unwrap_or(existing.name);
        let description = request.description.or(existing.description);
        let color = request.color.or(existing.color);
        let icon = request.icon.or(existing.icon);
        let sort_order = request.sort_order.unwrap_or(existing.sort_order);
        let is_active = request.is_active.unwrap_or(existing.is_active);

        sqlx::query(
            r#"
            UPDATE announcement_types
            SET name = ?, description = ?, color = ?, icon = ?,
                sort_order = ?, is_active = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
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
        .map_err(|e| AppError::Database(e.to_string()))?;

        self.find_by_id(id)
            .await?
            .ok_or_else(|| AppError::Database("Failed to retrieve updated announcement type".to_string()))
    }

    async fn delete(&self, id: Uuid) -> Result<()> {
        let id_str = id.to_string();

        sqlx::query("DELETE FROM announcement_types WHERE id = ?")
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
            FROM announcements
            WHERE announcement_type_id = ?
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
            "SELECT MAX(sort_order) FROM announcement_types"
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(row.0.unwrap_or(0) + 1)
    }

    async fn reorder(&self, ids: &[Uuid]) -> Result<()> {
        for (index, id) in ids.iter().enumerate() {
            let id_str = id.to_string();
            sqlx::query("UPDATE announcement_types SET sort_order = ? WHERE id = ?")
                .bind(index as i32)
                .bind(&id_str)
                .execute(&self.pool)
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;
        }
        Ok(())
    }

    async fn seed_defaults(&self) -> Result<Vec<AnnouncementTypeConfig>> {
        let defaults = default_announcement_types();
        let mut created = Vec::new();

        for (index, (name, slug, color)) in defaults.into_iter().enumerate() {
            // Skip if already exists
            if self.find_by_slug(slug).await?.is_some() {
                continue;
            }

            let id = Uuid::new_v4();
            let id_str = id.to_string();
            let now = Utc::now().naive_utc();

            sqlx::query(
                r#"
                INSERT INTO announcement_types (
                    id, name, slug, description, color, icon,
                    sort_order, is_active, is_system, created_at, updated_at
                ) VALUES (?, ?, ?, NULL, ?, NULL, ?, 1, 1, ?, ?)
                "#,
            )
            .bind(&id_str)
            .bind(name)
            .bind(slug)
            .bind(color)
            .bind(index as i32)
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
