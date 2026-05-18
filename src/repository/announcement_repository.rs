use async_trait::async_trait;
use chrono::{DateTime, Utc, NaiveDateTime};
use sqlx::{SqlitePool, FromRow};
use uuid::Uuid;

use crate::{
    domain::{Announcement, AnnouncementType},
    error::{AppError, Result},
};

#[async_trait]
pub trait AnnouncementRepository: Send + Sync {
    async fn create(&self, announcement: Announcement) -> Result<Announcement>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<Announcement>>;
    async fn list(&self, limit: i64, offset: i64) -> Result<Vec<Announcement>>;
    async fn list_recent(&self, limit: i64) -> Result<Vec<Announcement>>;
    async fn list_public(&self) -> Result<Vec<Announcement>>;
    async fn count_private_published(&self) -> Result<i64>;
    async fn update(&self, id: Uuid, announcement: Announcement) -> Result<Announcement>;
    async fn delete(&self, id: Uuid) -> Result<()>;
    /// Draft rows whose `scheduled_publish_at <= now`. Used by the
    /// background runner to find rows ready to auto-publish.
    async fn list_due_for_publish(&self, now: DateTime<Utc>) -> Result<Vec<Announcement>>;
    /// Atomic Draft→Published transition. Returns `true` iff a row
    /// was claimed (status was still Draft); `false` if someone else
    /// already flipped it. Used by the runner to avoid double-dispatch.
    async fn mark_published_now(&self, id: Uuid) -> Result<bool>;
}

#[derive(FromRow)]
struct AnnouncementRow {
    id: String,
    title: String,
    content: String,
    announcement_type: String,
    announcement_type_id: Option<String>,
    is_public: i32,
    featured: i32,
    image_url: Option<String>,
    published_at: Option<NaiveDateTime>,
    scheduled_publish_at: Option<NaiveDateTime>,
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
        let announcement_type_id = row.announcement_type_id
            .as_ref()
            .map(|id| Uuid::parse_str(id))
            .transpose()
            .map_err(|e| AppError::Internal(e.to_string()))?;

        Ok(Announcement {
            id: Uuid::parse_str(&row.id).map_err(|e| AppError::Internal(e.to_string()))?,
            title: row.title,
            content: row.content,
            announcement_type: Self::parse_announcement_type(&row.announcement_type)?,
            announcement_type_id,
            is_public: row.is_public != 0,
            featured: row.featured != 0,
            image_url: row.image_url,
            published_at: row.published_at.map(|dt| DateTime::from_naive_utc_and_offset(dt, Utc)),
            scheduled_publish_at: row.scheduled_publish_at.map(|dt| DateTime::from_naive_utc_and_offset(dt, Utc)),
            created_by: Uuid::parse_str(&row.created_by).map_err(|e| AppError::Internal(e.to_string()))?,
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
            _ => Err(AppError::Internal(format!("Invalid announcement type: {}", s))),
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
        let announcement_type_id_str = announcement.announcement_type_id.map(|id| id.to_string());
        let is_public_int = if announcement.is_public { 1i32 } else { 0i32 };
        let featured_int = if announcement.featured { 1i32 } else { 0i32 };
        let published_at_naive = announcement.published_at.map(|dt| dt.naive_utc());
        let scheduled_publish_at_naive = announcement.scheduled_publish_at.map(|dt| dt.naive_utc());
        let created_by_str = announcement.created_by.to_string();
        let now = Utc::now().naive_utc();

        sqlx::query(
            r#"
            INSERT INTO announcements (
                id, title, content, announcement_type, announcement_type_id, is_public, featured,
                image_url, published_at, scheduled_publish_at, created_by, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#
        )
        .bind(&id_str)
        .bind(&announcement.title)
        .bind(&announcement.content)
        .bind(announcement_type_str)
        .bind(&announcement_type_id_str)
        .bind(is_public_int)
        .bind(featured_int)
        .bind(&announcement.image_url)
        .bind(published_at_naive)
        .bind(scheduled_publish_at_naive)
        .bind(&created_by_str)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;

        self.find_by_id(announcement.id).await?.ok_or_else(|| {
            AppError::Internal("Failed to retrieve created announcement".to_string())
        })
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<Announcement>> {
        let id_str = id.to_string();
        let row = sqlx::query_as::<_, AnnouncementRow>(
            r#"
            SELECT id, title, content, announcement_type, announcement_type_id, is_public, featured,
                   image_url, published_at, scheduled_publish_at, created_by, created_at, updated_at
            FROM announcements
            WHERE id = ?
            "#
        )
        .bind(id_str)
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        match row {
            Some(r) => Ok(Some(Self::row_to_announcement(r)?)),
            None => Ok(None)
        }
    }

    async fn list(&self, limit: i64, offset: i64) -> Result<Vec<Announcement>> {
        let rows = sqlx::query_as::<_, AnnouncementRow>(
            r#"
            SELECT id, title, content, announcement_type, announcement_type_id, is_public, featured,
                   image_url, published_at, scheduled_publish_at, created_by, created_at, updated_at
            FROM announcements
            ORDER BY created_at DESC
            LIMIT ? OFFSET ?
            "#
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        rows.into_iter()
            .map(Self::row_to_announcement)
            .collect()
    }

    async fn list_recent(&self, limit: i64) -> Result<Vec<Announcement>> {
        let rows = sqlx::query_as::<_, AnnouncementRow>(
            r#"
            SELECT id, title, content, announcement_type, announcement_type_id, is_public, featured,
                   image_url, published_at, scheduled_publish_at, created_by, created_at, updated_at
            FROM announcements
            WHERE published_at IS NOT NULL
            ORDER BY published_at DESC
            LIMIT ?
            "#
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        rows.into_iter()
            .map(Self::row_to_announcement)
            .collect()
    }

    async fn list_public(&self) -> Result<Vec<Announcement>> {
        let rows = sqlx::query_as::<_, AnnouncementRow>(
            r#"
            SELECT id, title, content, announcement_type, announcement_type_id, is_public, featured,
                   image_url, published_at, scheduled_publish_at, created_by, created_at, updated_at
            FROM announcements
            WHERE is_public = 1 AND published_at IS NOT NULL
            ORDER BY published_at DESC
            "#
        )
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        rows.into_iter()
            .map(Self::row_to_announcement)
            .collect()
    }

    async fn count_private_published(&self) -> Result<i64> {
        let count: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*) as count
            FROM announcements
            WHERE is_public = 0 AND published_at IS NOT NULL
            "#
        )
        .fetch_one(&self.pool)
        .await
        .map_err(AppError::Database)?;

        Ok(count.0)
    }

    async fn update(&self, id: Uuid, announcement: Announcement) -> Result<Announcement> {
        let id_str = id.to_string();
        let announcement_type_str = Self::announcement_type_to_str(&announcement.announcement_type);
        let announcement_type_id_str = announcement.announcement_type_id.map(|id| id.to_string());
        let is_public_int = if announcement.is_public { 1i32 } else { 0i32 };
        let featured_int = if announcement.featured { 1i32 } else { 0i32 };
        let published_at_naive = announcement.published_at.map(|dt| dt.naive_utc());
        let scheduled_publish_at_naive = announcement.scheduled_publish_at.map(|dt| dt.naive_utc());
        let now = Utc::now().naive_utc();

        sqlx::query(
            r#"
            UPDATE announcements
            SET title = ?, content = ?, announcement_type = ?, announcement_type_id = ?,
                is_public = ?, featured = ?, image_url = ?, published_at = ?,
                scheduled_publish_at = ?, updated_at = ?
            WHERE id = ?
            "#
        )
        .bind(&announcement.title)
        .bind(&announcement.content)
        .bind(announcement_type_str)
        .bind(&announcement_type_id_str)
        .bind(is_public_int)
        .bind(featured_int)
        .bind(&announcement.image_url)
        .bind(published_at_naive)
        .bind(scheduled_publish_at_naive)
        .bind(now)
        .bind(&id_str)
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;

        self.find_by_id(id).await?.ok_or_else(|| {
            AppError::Internal("Failed to retrieve updated announcement".to_string())
        })
    }

    async fn delete(&self, id: Uuid) -> Result<()> {
        let id_str = id.to_string();
        sqlx::query("DELETE FROM announcements WHERE id = ?")
            .bind(&id_str)
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;

        Ok(())
    }

    async fn list_due_for_publish(&self, now: DateTime<Utc>) -> Result<Vec<Announcement>> {
        let now_naive = now.naive_utc();
        let rows = sqlx::query_as::<_, AnnouncementRow>(
            r#"
            SELECT id, title, content, announcement_type, announcement_type_id, is_public, featured,
                   image_url, published_at, scheduled_publish_at, created_by, created_at, updated_at
            FROM announcements
            WHERE published_at IS NULL
              AND scheduled_publish_at IS NOT NULL
              AND scheduled_publish_at <= ?
            ORDER BY scheduled_publish_at ASC
            "#
        )
        .bind(now_naive)
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        rows.into_iter()
            .map(Self::row_to_announcement)
            .collect()
    }

    async fn mark_published_now(&self, id: Uuid) -> Result<bool> {
        let id_str = id.to_string();
        let now = Utc::now().naive_utc();
        // Conditional UPDATE: only flips a row that is still a Draft
        // (published_at IS NULL). The atomicity is what prevents two
        // concurrent runner ticks from both dispatching the integration
        // event — exactly one wins and gets a non-zero row count.
        let result = sqlx::query(
            r#"
            UPDATE announcements
            SET published_at = ?, scheduled_publish_at = NULL, updated_at = ?
            WHERE id = ? AND published_at IS NULL
            "#
        )
        .bind(now)
        .bind(now)
        .bind(&id_str)
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;

        Ok(result.rows_affected() > 0)
    }
}