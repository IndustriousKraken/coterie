use async_trait::async_trait;
use chrono::{DateTime, Utc, NaiveDateTime};
use sqlx::{SqlitePool, FromRow};
use uuid::Uuid;

use crate::{
    domain::{AttendanceStatus, Event, EventType, EventVisibility},
    error::{AppError, Result},
    repository::EventRepository,
};

#[derive(FromRow)]
struct EventRow {
    id: String,
    title: String,
    description: String,
    event_type: String,
    event_type_id: Option<String>,
    visibility: String,
    start_time: NaiveDateTime,
    end_time: Option<NaiveDateTime>,
    location: Option<String>,
    max_attendees: Option<i32>,
    rsvp_required: i32,
    image_url: Option<String>,
    created_by: String,
    created_at: NaiveDateTime,
    updated_at: NaiveDateTime,
}

pub struct SqliteEventRepository {
    pool: SqlitePool,
}

impl SqliteEventRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    fn row_to_event(row: EventRow) -> Result<Event> {
        let event_type_id = row.event_type_id
            .as_ref()
            .map(|id| Uuid::parse_str(id))
            .transpose()
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(Event {
            id: Uuid::parse_str(&row.id).map_err(|e| AppError::Database(e.to_string()))?,
            title: row.title,
            description: row.description,
            event_type: Self::parse_event_type(&row.event_type)?,
            event_type_id,
            visibility: Self::parse_visibility(&row.visibility)?,
            start_time: DateTime::from_naive_utc_and_offset(row.start_time, Utc),
            end_time: row.end_time.map(|dt| DateTime::from_naive_utc_and_offset(dt, Utc)),
            location: row.location,
            max_attendees: row.max_attendees,
            rsvp_required: row.rsvp_required != 0,
            image_url: row.image_url,
            created_by: Uuid::parse_str(&row.created_by).map_err(|e| AppError::Database(e.to_string()))?,
            created_at: DateTime::from_naive_utc_and_offset(row.created_at, Utc),
            updated_at: DateTime::from_naive_utc_and_offset(row.updated_at, Utc),
        })
    }

    fn parse_event_type(s: &str) -> Result<EventType> {
        match s {
            "Meeting" => Ok(EventType::Meeting),
            "Workshop" => Ok(EventType::Workshop),
            "CTF" => Ok(EventType::CTF),
            "Social" => Ok(EventType::Social),
            "Training" => Ok(EventType::Training),
            "Hackathon" => Ok(EventType::Hackathon),
            _ => Err(AppError::Database(format!("Invalid event type: {}", s))),
        }
    }

    fn event_type_to_str(event_type: &EventType) -> &'static str {
        match event_type {
            EventType::Meeting => "Meeting",
            EventType::Workshop => "Workshop",
            EventType::CTF => "CTF",
            EventType::Social => "Social",
            EventType::Training => "Training",
            EventType::Hackathon => "Hackathon",
        }
    }

    fn parse_visibility(s: &str) -> Result<EventVisibility> {
        match s {
            "Public" => Ok(EventVisibility::Public),
            "MembersOnly" => Ok(EventVisibility::MembersOnly),
            "AdminOnly" => Ok(EventVisibility::AdminOnly),
            _ => Err(AppError::Database(format!("Invalid visibility: {}", s))),
        }
    }

    fn visibility_to_str(visibility: &EventVisibility) -> &'static str {
        match visibility {
            EventVisibility::Public => "Public",
            EventVisibility::MembersOnly => "MembersOnly",
            EventVisibility::AdminOnly => "AdminOnly",
        }
    }
}

#[async_trait]
impl EventRepository for SqliteEventRepository {
    async fn create(&self, event: Event) -> Result<Event> {
        let id_str = event.id.to_string();
        let event_type_str = Self::event_type_to_str(&event.event_type);
        let event_type_id_str = event.event_type_id.map(|id| id.to_string());
        let visibility_str = Self::visibility_to_str(&event.visibility);
        let start_time_naive = event.start_time.naive_utc();
        let end_time_naive = event.end_time.map(|dt| dt.naive_utc());
        let max_attendees_int = event.max_attendees;
        let rsvp_required_int = if event.rsvp_required { 1i32 } else { 0i32 };
        let created_by_str = event.created_by.to_string();
        let now = Utc::now().naive_utc();

        sqlx::query(
            r#"
            INSERT INTO events (
                id, title, description, event_type, event_type_id, visibility,
                start_time, end_time, location, max_attendees, rsvp_required,
                image_url, created_by, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#
        )
        .bind(&id_str)
        .bind(&event.title)
        .bind(&event.description)
        .bind(event_type_str)
        .bind(&event_type_id_str)
        .bind(visibility_str)
        .bind(start_time_naive)
        .bind(end_time_naive)
        .bind(&event.location)
        .bind(max_attendees_int)
        .bind(rsvp_required_int)
        .bind(&event.image_url)
        .bind(&created_by_str)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        self.find_by_id(event.id).await?.ok_or_else(|| {
            AppError::Database("Failed to retrieve created event".to_string())
        })
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<Event>> {
        let id_str = id.to_string();
        let row = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT id, title, description, event_type, event_type_id, visibility,
                   start_time, end_time, location, max_attendees, rsvp_required,
                   image_url, created_by, created_at, updated_at
            FROM events
            WHERE id = ?
            "#
        )
        .bind(id_str)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        match row {
            Some(r) => Ok(Some(Self::row_to_event(r)?)),
            None => Ok(None)
        }
    }

    async fn list(&self, limit: i64, offset: i64) -> Result<Vec<Event>> {
        let rows = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT id, title, description, event_type, event_type_id, visibility,
                   start_time, end_time, location, max_attendees, rsvp_required,
                   image_url, created_by, created_at, updated_at
            FROM events
            ORDER BY start_time DESC
            LIMIT ? OFFSET ?
            "#
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        rows.into_iter()
            .map(Self::row_to_event)
            .collect()
    }

    async fn list_upcoming(&self, limit: i64) -> Result<Vec<Event>> {
        let now = Utc::now().naive_utc();

        let rows = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT id, title, description, event_type, event_type_id, visibility,
                   start_time, end_time, location, max_attendees, rsvp_required,
                   image_url, created_by, created_at, updated_at
            FROM events
            WHERE start_time > ?
            ORDER BY start_time ASC
            LIMIT ?
            "#
        )
        .bind(now)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        rows.into_iter()
            .map(Self::row_to_event)
            .collect()
    }

    async fn list_public(&self) -> Result<Vec<Event>> {
        let visibility_str = Self::visibility_to_str(&EventVisibility::Public);

        let rows = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT id, title, description, event_type, event_type_id, visibility,
                   start_time, end_time, location, max_attendees, rsvp_required,
                   image_url, created_by, created_at, updated_at
            FROM events
            WHERE visibility = ?
            ORDER BY start_time DESC
            "#
        )
        .bind(visibility_str)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        rows.into_iter()
            .map(Self::row_to_event)
            .collect()
    }

    async fn update(&self, id: Uuid, event: Event) -> Result<Event> {
        let id_str = id.to_string();
        let event_type_str = Self::event_type_to_str(&event.event_type);
        let event_type_id_str = event.event_type_id.map(|id| id.to_string());
        let visibility_str = Self::visibility_to_str(&event.visibility);
        let start_time_naive = event.start_time.naive_utc();
        let end_time_naive = event.end_time.map(|dt| dt.naive_utc());
        let max_attendees_int = event.max_attendees;
        let rsvp_required_int = if event.rsvp_required { 1i32 } else { 0i32 };
        let now = Utc::now().naive_utc();

        sqlx::query(
            r#"
            UPDATE events
            SET title = ?, description = ?, event_type = ?, event_type_id = ?, visibility = ?,
                start_time = ?, end_time = ?, location = ?, max_attendees = ?,
                rsvp_required = ?, image_url = ?, updated_at = ?
            WHERE id = ?
            "#
        )
        .bind(&event.title)
        .bind(&event.description)
        .bind(event_type_str)
        .bind(&event_type_id_str)
        .bind(visibility_str)
        .bind(start_time_naive)
        .bind(end_time_naive)
        .bind(&event.location)
        .bind(max_attendees_int)
        .bind(rsvp_required_int)
        .bind(&event.image_url)
        .bind(now)
        .bind(&id_str)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        self.find_by_id(id).await?.ok_or_else(|| {
            AppError::Database("Failed to retrieve updated event".to_string())
        })
    }

    async fn delete(&self, id: Uuid) -> Result<()> {
        let id_str = id.to_string();
        sqlx::query("DELETE FROM events WHERE id = ?")
            .bind(&id_str)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }

    async fn register_attendance(&self, event_id: Uuid, member_id: Uuid) -> Result<()> {
        let event_id_str = event_id.to_string();
        let member_id_str = member_id.to_string();
        
        sqlx::query(
            r#"
            INSERT INTO event_attendance (event_id, member_id, status, registered_at)
            VALUES (?, ?, 'Registered', CURRENT_TIMESTAMP)
            ON CONFLICT (event_id, member_id) 
            DO UPDATE SET status = 'Registered', registered_at = CURRENT_TIMESTAMP
            "#
        )
        .bind(&event_id_str)
        .bind(&member_id_str)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }

    async fn cancel_attendance(&self, event_id: Uuid, member_id: Uuid) -> Result<()> {
        let event_id_str = event_id.to_string();
        let member_id_str = member_id.to_string();

        sqlx::query(
            r#"
            UPDATE event_attendance
            SET status = 'Cancelled'
            WHERE event_id = ? AND member_id = ?
            "#
        )
        .bind(&event_id_str)
        .bind(&member_id_str)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }

    async fn get_attendee_count(&self, event_id: Uuid) -> Result<i64> {
        let event_id_str = event_id.to_string();

        let row: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*) as count
            FROM event_attendance
            WHERE event_id = ? AND status = 'Registered'
            "#
        )
        .bind(&event_id_str)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(row.0)
    }

    async fn get_member_attendance_status(&self, event_id: Uuid, member_id: Uuid) -> Result<Option<AttendanceStatus>> {
        let event_id_str = event_id.to_string();
        let member_id_str = member_id.to_string();

        let row: Option<(String,)> = sqlx::query_as(
            r#"
            SELECT status
            FROM event_attendance
            WHERE event_id = ? AND member_id = ?
            "#
        )
        .bind(&event_id_str)
        .bind(&member_id_str)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        match row {
            Some((status,)) => {
                let attendance_status = match status.as_str() {
                    "Registered" => AttendanceStatus::Registered,
                    "Waitlisted" => AttendanceStatus::Waitlisted,
                    "Cancelled" => AttendanceStatus::Cancelled,
                    _ => return Err(AppError::Database(format!("Invalid attendance status: {}", status))),
                };
                Ok(Some(attendance_status))
            }
            None => Ok(None),
        }
    }

    async fn get_member_registered_events(&self, member_id: Uuid) -> Result<Vec<Event>> {
        let member_id_str = member_id.to_string();
        let now = Utc::now().naive_utc();

        let rows = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT e.id, e.title, e.description, e.event_type, e.event_type_id, e.visibility,
                   e.start_time, e.end_time, e.location, e.max_attendees, e.rsvp_required,
                   e.created_by, e.created_at, e.updated_at
            FROM events e
            INNER JOIN event_attendance ea ON e.id = ea.event_id
            WHERE ea.member_id = ? AND ea.status = 'Registered' AND e.start_time > ?
            ORDER BY e.start_time ASC
            "#
        )
        .bind(&member_id_str)
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        rows.into_iter()
            .map(Self::row_to_event)
            .collect()
    }
}