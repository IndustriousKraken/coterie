use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Announcement {
    pub id: Uuid,
    pub title: String,
    pub content: String,
    pub announcement_type: AnnouncementType,
    pub announcement_type_id: Option<Uuid>,
    pub is_public: bool,
    pub featured: bool,
    pub published_at: Option<DateTime<Utc>>,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Legacy announcement type enum - DEPRECATED
///
/// This enum is being phased out in favor of database-driven announcement types.
/// Use `announcement_type_id` field to reference `AnnouncementTypeConfig` from the
/// `announcement_types` table instead.
///
/// To get the announcement type name, look up the type by ID:
/// ```ignore
/// let type_config = announcement_type_service.get(announcement.announcement_type_id).await?;
/// let type_name = type_config.name;
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT")]
pub enum AnnouncementType {
    News,
    Achievement,
    Meeting,
    CTFResult,
    General,
}