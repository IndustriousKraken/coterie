use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
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
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT")]
pub enum EventType {
    Meeting,
    Workshop,
    CTF,
    Social,
    Training,
    Hackathon,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
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