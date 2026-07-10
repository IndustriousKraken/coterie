use chrono::{DateTime, Utc};
use chrono_tz::{OffsetName, Tz};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::wall_clock_to_utc;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct Announcement {
    pub id: Uuid,
    pub title: String,
    pub content: String,
    pub announcement_type: AnnouncementType,
    pub announcement_type_id: Option<Uuid>,
    pub is_public: bool,
    pub featured: bool,
    pub image_url: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    /// The scheduled-publish **local wall-clock** — NOT a real UTC
    /// instant. Its naive component is what the admin typed; it is
    /// paired with [`Announcement::scheduled_publish_timezone`] to
    /// derive the actual publish instant via
    /// [`Announcement::scheduled_publish_utc`]. Kept in a
    /// `DateTime<Utc>` only as a naive container (the DB column is a
    /// naive DATETIME). Mirrors `Event::start_time`.
    pub scheduled_publish_at: Option<DateTime<Utc>>,
    /// IANA zone name (e.g. `America/New_York`) the scheduled wall-clock
    /// is understood in. Frozen from `org.timezone` at scheduling.
    pub scheduled_publish_timezone: String,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Announcement {
    /// The frozen IANA zone the scheduled wall-clock is understood in,
    /// falling back to UTC when the stored name is empty/unrecognized.
    pub fn scheduled_tz(&self) -> Tz {
        self.scheduled_publish_timezone.parse().unwrap_or(Tz::UTC)
    }

    /// Derive the true UTC publish instant from the stored (wall-clock,
    /// zone). `None` when not scheduled. Recomputed on every read, so a
    /// later change to the zone's rules is picked up automatically.
    /// Mirrors `Event::start_utc`; the runner compares THIS to `now`.
    pub fn scheduled_publish_utc(&self) -> Option<DateTime<Utc>> {
        self.scheduled_publish_at
            .map(|dt| wall_clock_to_utc(dt.naive_utc(), self.scheduled_tz()))
    }

    /// The scheduled time's zone abbreviation at its wall-clock (e.g.
    /// `EDT` / `EST` / `UTC`), for labeling the admin surface so the
    /// scheduled time isn't mislabeled "UTC". `None` when not scheduled.
    pub fn scheduled_zone_abbr(&self) -> Option<String> {
        self.scheduled_publish_utc().map(|utc| {
            utc.with_timezone(&self.scheduled_tz())
                .offset()
                .abbreviation()
                .unwrap_or("UTC")
                .to_string()
        })
    }
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
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, ToSchema)]
#[sqlx(type_name = "TEXT")]
pub enum AnnouncementType {
    News,
    Achievement,
    Meeting,
    CTFResult,
    General,
}
