use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Event {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub event_type: EventType,
    pub visibility: EventVisibility,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub location: Option<String>,
    pub max_attendees: Option<i32>,
    pub rsvp_required: bool,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

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