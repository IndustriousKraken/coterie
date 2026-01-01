use async_trait::async_trait;
use chrono::{DateTime, Utc, NaiveDateTime};
use sqlx::{SqlitePool, FromRow};
use uuid::Uuid;

use crate::{
    domain::{Announcement, AnnouncementType},
    error::{AppError, Result},
    repository::AnnouncementRepository,
};

#[derive(FromRow)]
struct AnnouncementRow {
    id: String,
    title: String,
    content: String,
    announcement_type: String,
    is_public: i32,
    featured: i32,
    published_at: Option<NaiveDateTime>,
    created_by: String,
    created_at: NaiveDateTime,
    updated_at: NaiveDateTime,
}

pub struct SqliteAnnouncementRepository {
    pool: SqlitePool,
}

impl SqliteAnnouncementRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    fn row_to_announcement(row: AnnouncementRow) -> Result<Announcement> {
        Ok(Announcement {
            id: Uuid::parse_str(&row.id).map_err(|e| AppError::Database(e.to_string()))?,
            title: row.title,
            content: row.content,
            announcement_type: Self::parse_announcement_type(&row.announcement_type)?,
            is_public: row.is_public != 0,
            featured: row.featured != 0,
            published_at: row.published_at.map(|dt| DateTime::from_naive_utc_and_offset(dt, Utc)),
            created_by: Uuid::parse_str(&row.created_by).map_err(|e| AppError::Database(e.to_string()))?,
            created_at: DateTime::from_naive_utc_and_offset(row.created_at, Utc),
            updated_at: DateTime::from_naive_utc_and_offset(row.updated_at, Utc),
        })
    }

    fn parse_announcement_type(s: &str) -> Result<AnnouncementType> {
        match s {
            "News" => Ok(AnnouncementType::News),
            "Achievement" => Ok(AnnouncementType::Achievement),
            "Meeting" => Ok(AnnouncementType::Meeting),
            "CTFResult" => Ok(AnnouncementType::CTFResult),
            "General" => Ok(AnnouncementType::General),
            _ => Err(AppError::Database(format!("Invalid announcement type: {}", s))),
        }
    }

    fn announcement_type_to_str(announcement_type: &AnnouncementType) -> &'static str {
        match announcement_type {
            AnnouncementType::News => "News",
            AnnouncementType::Achievement => "Achievement",
            AnnouncementType::Meeting => "Meeting",
            AnnouncementType::CTFResult => "CTFResult",
            AnnouncementType::General => "General",
        }
    }
}

#[async_trait]
impl AnnouncementRepository for SqliteAnnouncementRepository {
    async fn create(&self, announcement: Announcement) -> Result<Announcement> {
        let id_str = announcement.id.to_string();
        let announcement_type_str = Self::announcement_type_to_str(&announcement.announcement_type);
        let is_public_int = if announcement.is_public { 1i32 } else { 0i32 };
        let featured_int = if announcement.featured { 1i32 } else { 0i32 };
        let published_at_naive = announcement.published_at.map(|dt| dt.naive_utc());
        let created_by_str = announcement.created_by.to_string();
        let now = Utc::now().naive_utc();

        sqlx::query(
            r#"
            INSERT INTO announcements (
                id, title, content, announcement_type, is_public, featured,
                published_at, created_by, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#
        )
        .bind(&id_str)
        .bind(&announcement.title)
        .bind(&announcement.content)
        .bind(announcement_type_str)
        .bind(is_public_int)
        .bind(featured_int)
        .bind(published_at_naive)
        .bind(&created_by_str)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        self.find_by_id(announcement.id).await?.ok_or_else(|| {
            AppError::Database("Failed to retrieve created announcement".to_string())
        })
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<Announcement>> {
        let id_str = id.to_string();
        let row = sqlx::query_as::<_, AnnouncementRow>(
            r#"
            SELECT id, title, content, announcement_type, is_public, featured,
                   published_at, created_by, created_at, updated_at
            FROM announcements
            WHERE id = ?
            "#
        )
        .bind(id_str)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        match row {
            Some(r) => Ok(Some(Self::row_to_announcement(r)?)),
            None => Ok(None)
        }
    }

    async fn list(&self, limit: i64, offset: i64) -> Result<Vec<Announcement>> {
        let rows = sqlx::query_as::<_, AnnouncementRow>(
            r#"
            SELECT id, title, content, announcement_type, is_public, featured,
                   published_at, created_by, created_at, updated_at
            FROM announcements
            ORDER BY created_at DESC
            LIMIT ? OFFSET ?
            "#
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        rows.into_iter()
            .map(Self::row_to_announcement)
            .collect()
    }

    async fn list_recent(&self, limit: i64) -> Result<Vec<Announcement>> {
        let rows = sqlx::query_as::<_, AnnouncementRow>(
            r#"
            SELECT id, title, content, announcement_type, is_public, featured,
                   published_at, created_by, created_at, updated_at
            FROM announcements
            WHERE published_at IS NOT NULL
            ORDER BY published_at DESC
            LIMIT ?
            "#
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        rows.into_iter()
            .map(Self::row_to_announcement)
            .collect()
    }

    async fn list_public(&self) -> Result<Vec<Announcement>> {
        let rows = sqlx::query_as::<_, AnnouncementRow>(
            r#"
            SELECT id, title, content, announcement_type, is_public, featured,
                   published_at, created_by, created_at, updated_at
            FROM announcements
            WHERE is_public = 1 AND published_at IS NOT NULL
            ORDER BY published_at DESC
            "#
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        rows.into_iter()
            .map(Self::row_to_announcement)
            .collect()
    }

    async fn list_featured(&self) -> Result<Vec<Announcement>> {
        let rows = sqlx::query_as::<_, AnnouncementRow>(
            r#"
            SELECT id, title, content, announcement_type, is_public, featured,
                   published_at, created_by, created_at, updated_at
            FROM announcements
            WHERE featured = 1 AND published_at IS NOT NULL
            ORDER BY published_at DESC
            "#
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        rows.into_iter()
            .map(Self::row_to_announcement)
            .collect()
    }

    async fn update(&self, id: Uuid, announcement: Announcement) -> Result<Announcement> {
        let id_str = id.to_string();
        let announcement_type_str = Self::announcement_type_to_str(&announcement.announcement_type);
        let is_public_int = if announcement.is_public { 1i32 } else { 0i32 };
        let featured_int = if announcement.featured { 1i32 } else { 0i32 };
        let published_at_naive = announcement.published_at.map(|dt| dt.naive_utc());
        let now = Utc::now().naive_utc();

        sqlx::query(
            r#"
            UPDATE announcements
            SET title = ?, content = ?, announcement_type = ?, 
                is_public = ?, featured = ?, published_at = ?,
                updated_at = ?
            WHERE id = ?
            "#
        )
        .bind(&announcement.title)
        .bind(&announcement.content)
        .bind(announcement_type_str)
        .bind(is_public_int)
        .bind(featured_int)
        .bind(published_at_naive)
        .bind(now)
        .bind(&id_str)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        self.find_by_id(id).await?.ok_or_else(|| {
            AppError::Database("Failed to retrieve updated announcement".to_string())
        })
    }

    async fn delete(&self, id: Uuid) -> Result<()> {
        let id_str = id.to_string();
        sqlx::query("DELETE FROM announcements WHERE id = ?")
            .bind(&id_str)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }
}