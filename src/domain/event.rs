use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct Event {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub event_type: EventType,
    pub event_type_id: Option<Uuid>,
    pub visibility: EventVisibility,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub location: Option<String>,
    pub max_attendees: Option<i32>,
    pub rsvp_required: bool,
    pub image_url: Option<String>,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// When set, this event is one occurrence of a recurring series.
    /// `None` for one-off events. The series row holds the recurrence
    /// rule + materialization horizon; per-occurrence data lives on
    /// this row.
    pub series_id: Option<Uuid>,
    /// 1-based position within the series, or `None` for one-offs.
    /// Used for display ("session 5 of 12") and stable ordering.
    pub occurrence_index: Option<i32>,
}

/// Persisted recurring-event series. The actual recurrence rule lives
/// in `rule_json` (a serialized [`crate::domain::Recurrence`]); the
/// `kind` mirrors that rule's discriminator for SQL filtering without
/// JSON parsing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventSeries {
    pub id: Uuid,
    pub rule_kind: String,
    pub rule_json: String,
    /// Optional last-occurrence cutoff. `None` = open-ended series.
    pub until_date: Option<DateTime<Utc>>,
    /// Latest occurrence start_time materialized into `events`. The
    /// daily horizon-extension job rolls this forward; on creation
    /// we materialize 12 months ahead.
    pub materialized_through: DateTime<Utc>,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Legacy event type enum - DEPRECATED
///
/// This enum is being phased out in favor of database-driven event types.
/// Use `event_type_id` field to reference `EventTypeConfig` from the
/// `event_types` table instead.
///
/// To get the event type name, look up the type by ID:
/// ```ignore
/// let type_config = event_type_service.get(event.event_type_id).await?;
/// let type_name = type_config.name;
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, ToSchema)]
#[sqlx(type_name = "TEXT")]
pub enum EventType {
    Meeting,
    Workshop,
    CTF,
    Social,
    Training,
    Hackathon,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, ToSchema)]
#[sqlx(type_name = "TEXT")]
pub enum EventVisibility {
    Public,
    MembersOnly,
    AdminOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventAttendance {
    pub event_id: Uuid,
    pub member_id: Uuid,
    pub status: AttendanceStatus,
    pub registered_at: DateTime<Utc>,
    pub attended: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT")]
pub enum AttendanceStatus {
    Registered,
    Waitlisted,
    Cancelled,
}