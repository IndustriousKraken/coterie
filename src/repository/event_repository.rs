use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

use crate::{
    domain::{AttendanceStatus, Event, EventType, EventVisibility},
    error::{AppError, Result},
};

/// One candidate row for the event-reminder runner — a flat join of
/// the attendee, event, and member rows that the runner needs to
/// render and send a reminder. Kept narrow on purpose: only the
/// fields the template + claim step actually touch.
#[derive(Debug, Clone)]
pub struct EventReminderRow {
    pub event_id: Uuid,
    pub event_title: String,
    pub event_start: DateTime<Utc>,
    pub event_location: Option<String>,
    pub member_id: Uuid,
    pub member_email: String,
    pub member_full_name: String,
}

#[async_trait]
pub trait EventRepository: Send + Sync {
    async fn create(&self, event: Event) -> Result<Event>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<Event>>;
    async fn list(&self, limit: i64, offset: i64) -> Result<Vec<Event>>;
    async fn list_upcoming(&self, limit: i64) -> Result<Vec<Event>>;
    async fn list_public(&self) -> Result<Vec<Event>>;
    async fn list_members_only(&self) -> Result<Vec<Event>>;
    async fn count_members_only_upcoming(&self) -> Result<i64>;
    async fn update(&self, id: Uuid, event: Event) -> Result<Event>;
    async fn delete(&self, id: Uuid) -> Result<()>;
    async fn register_attendance(&self, event_id: Uuid, member_id: Uuid) -> Result<()>;
    async fn cancel_attendance(&self, event_id: Uuid, member_id: Uuid) -> Result<()>;
    async fn get_attendee_count(&self, event_id: Uuid) -> Result<i64>;
    async fn get_member_attendance_status(
        &self,
        event_id: Uuid,
        member_id: Uuid,
    ) -> Result<Option<AttendanceStatus>>;

    // ---- Event-reminder support ---------------------------------------

    /// Candidate RSVPs whose event starts in `(now, until]`, are
    /// status='Registered', and haven't been reminded yet. The runner
    /// iterates this list and tries to atomically claim each via
    /// `mark_reminder_sent` before sending the email.
    async fn list_pending_reminders(
        &self,
        now: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> Result<Vec<EventReminderRow>>;
    /// Conditional UPDATE that stamps `reminder_sent_at` only if it
    /// was NULL — returns true exactly when a row was claimed. The
    /// runner uses this as a concurrency-safe lock before sending the
    /// email so two ticks (or two processes) can't double-send.
    async fn mark_reminder_sent(&self, event_id: Uuid, member_id: Uuid) -> Result<bool>;

    // ---- Recurring-series support -------------------------------------

    /// Highest `occurrence_index` already materialized for this series,
    /// or `None` if the series has no rows yet. Used by the materializer
    /// to continue numbering on horizon-extension passes.
    async fn max_occurrence_index_for_series(&self, series_id: Uuid) -> Result<Option<i32>>;
    /// Look up the concrete event row for a `(series_id, occurrence_index)`
    /// pair. Used by per-occurrence exception flows (cancel deletes this
    /// row, override updates it).
    async fn find_by_series_and_index(
        &self,
        series_id: Uuid,
        occurrence_index: i32,
    ) -> Result<Option<Event>>;
    /// Hard-delete every occurrence in the series whose `start_time`
    /// is strictly greater than `after`. Returns the count deleted.
    /// Used by "end the series after this date" and by the
    /// re-materialization safety net.
    async fn delete_series_occurrences_after(
        &self,
        series_id: Uuid,
        after: chrono::DateTime<chrono::Utc>,
    ) -> Result<u64>;
    /// Apply the editable subset of fields (title, description, type,
    /// visibility, location, max_attendees, rsvp_required) to every
    /// occurrence in the series whose `start_time >= from`. Used by
    /// the "edit this and all future" admin action — start_time and
    /// per-row image_url are deliberately preserved per occurrence.
    async fn update_series_occurrences_from(
        &self,
        series_id: Uuid,
        from: chrono::DateTime<chrono::Utc>,
        template: &Event,
    ) -> Result<u64>;
}

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
    series_id: Option<String>,
    occurrence_index: Option<i32>,
}

pub struct SqliteEventRepository {
    pool: SqlitePool,
}

impl SqliteEventRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    fn row_to_event(row: EventRow) -> Result<Event> {
        let event_type_id = row
            .event_type_id
            .as_ref()
            .map(|id| Uuid::parse_str(id))
            .transpose()
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let series_id = row
            .series_id
            .as_ref()
            .map(|id| Uuid::parse_str(id))
            .transpose()
            .map_err(|e| AppError::Internal(e.to_string()))?;

        Ok(Event {
            id: Uuid::parse_str(&row.id).map_err(|e| AppError::Internal(e.to_string()))?,
            title: row.title,
            description: row.description,
            event_type: Self::parse_event_type(&row.event_type)?,
            event_type_id,
            visibility: Self::parse_visibility(&row.visibility)?,
            start_time: DateTime::from_naive_utc_and_offset(row.start_time, Utc),
            end_time: row
                .end_time
                .map(|dt| DateTime::from_naive_utc_and_offset(dt, Utc)),
            location: row.location,
            max_attendees: row.max_attendees,
            rsvp_required: row.rsvp_required != 0,
            image_url: row.image_url,
            created_by: Uuid::parse_str(&row.created_by)
                .map_err(|e| AppError::Internal(e.to_string()))?,
            created_at: DateTime::from_naive_utc_and_offset(row.created_at, Utc),
            updated_at: DateTime::from_naive_utc_and_offset(row.updated_at, Utc),
            series_id,
            occurrence_index: row.occurrence_index,
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
            _ => Err(AppError::Internal(format!("Invalid event type: {}", s))),
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
            _ => Err(AppError::Internal(format!("Invalid visibility: {}", s))),
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

        let series_id_str = event.series_id.map(|id| id.to_string());

        sqlx::query(
            r#"
            INSERT INTO events (
                id, title, description, event_type, event_type_id, visibility,
                start_time, end_time, location, max_attendees, rsvp_required,
                image_url, created_by, created_at, updated_at,
                series_id, occurrence_index
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
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
        .bind(&series_id_str)
        .bind(event.occurrence_index)
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;

        self.find_by_id(event.id)
            .await?
            .ok_or_else(|| AppError::Internal("Failed to retrieve created event".to_string()))
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<Event>> {
        let id_str = id.to_string();
        let row = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT id, title, description, event_type, event_type_id, visibility,
                   start_time, end_time, location, max_attendees, rsvp_required,
                   image_url, created_by, created_at, updated_at,
                   series_id, occurrence_index
            FROM events
            WHERE id = ?
            "#,
        )
        .bind(id_str)
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        match row {
            Some(r) => Ok(Some(Self::row_to_event(r)?)),
            None => Ok(None),
        }
    }

    async fn list(&self, limit: i64, offset: i64) -> Result<Vec<Event>> {
        let rows = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT id, title, description, event_type, event_type_id, visibility,
                   start_time, end_time, location, max_attendees, rsvp_required,
                   image_url, created_by, created_at, updated_at,
                   series_id, occurrence_index
            FROM events
            ORDER BY start_time DESC
            LIMIT ? OFFSET ?
            "#,
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        rows.into_iter().map(Self::row_to_event).collect()
    }

    async fn list_upcoming(&self, limit: i64) -> Result<Vec<Event>> {
        let now = Utc::now().naive_utc();

        let rows = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT id, title, description, event_type, event_type_id, visibility,
                   start_time, end_time, location, max_attendees, rsvp_required,
                   image_url, created_by, created_at, updated_at,
                   series_id, occurrence_index
            FROM events
            WHERE start_time > ?
            ORDER BY start_time ASC
            LIMIT ?
            "#,
        )
        .bind(now)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        rows.into_iter().map(Self::row_to_event).collect()
    }

    async fn list_public(&self) -> Result<Vec<Event>> {
        let visibility_str = Self::visibility_to_str(&EventVisibility::Public);

        let rows = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT id, title, description, event_type, event_type_id, visibility,
                   start_time, end_time, location, max_attendees, rsvp_required,
                   image_url, created_by, created_at, updated_at,
                   series_id, occurrence_index
            FROM events
            WHERE visibility = ?
            ORDER BY start_time DESC
            "#,
        )
        .bind(visibility_str)
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        rows.into_iter().map(Self::row_to_event).collect()
    }

    async fn list_members_only(&self) -> Result<Vec<Event>> {
        let visibility_str = Self::visibility_to_str(&EventVisibility::MembersOnly);

        let rows = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT id, title, description, event_type, event_type_id, visibility,
                   start_time, end_time, location, max_attendees, rsvp_required,
                   image_url, created_by, created_at, updated_at,
                   series_id, occurrence_index
            FROM events
            WHERE visibility = ?
            ORDER BY start_time DESC
            "#,
        )
        .bind(visibility_str)
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        rows.into_iter().map(Self::row_to_event).collect()
    }

    async fn count_members_only_upcoming(&self) -> Result<i64> {
        let visibility_str = Self::visibility_to_str(&EventVisibility::MembersOnly);
        let now = Utc::now().naive_utc();

        let count: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*) as count
            FROM events
            WHERE visibility = ? AND start_time > ?
            "#,
        )
        .bind(visibility_str)
        .bind(now)
        .fetch_one(&self.pool)
        .await
        .map_err(AppError::Database)?;

        Ok(count.0)
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
            "#,
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
        .map_err(AppError::Database)?;

        self.find_by_id(id)
            .await?
            .ok_or_else(|| AppError::Internal("Failed to retrieve updated event".to_string()))
    }

    async fn delete(&self, id: Uuid) -> Result<()> {
        let id_str = id.to_string();
        sqlx::query("DELETE FROM events WHERE id = ?")
            .bind(&id_str)
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;

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
            "#,
        )
        .bind(&event_id_str)
        .bind(&member_id_str)
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;

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
            "#,
        )
        .bind(&event_id_str)
        .bind(&member_id_str)
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;

        Ok(())
    }

    async fn get_attendee_count(&self, event_id: Uuid) -> Result<i64> {
        let event_id_str = event_id.to_string();

        let row: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*) as count
            FROM event_attendance
            WHERE event_id = ? AND status = 'Registered'
            "#,
        )
        .bind(&event_id_str)
        .fetch_one(&self.pool)
        .await
        .map_err(AppError::Database)?;

        Ok(row.0)
    }

    async fn get_member_attendance_status(
        &self,
        event_id: Uuid,
        member_id: Uuid,
    ) -> Result<Option<AttendanceStatus>> {
        let event_id_str = event_id.to_string();
        let member_id_str = member_id.to_string();

        let row: Option<(String,)> = sqlx::query_as(
            r#"
            SELECT status
            FROM event_attendance
            WHERE event_id = ? AND member_id = ?
            "#,
        )
        .bind(&event_id_str)
        .bind(&member_id_str)
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        match row {
            Some((status,)) => {
                let attendance_status = match status.as_str() {
                    "Registered" => AttendanceStatus::Registered,
                    "Waitlisted" => AttendanceStatus::Waitlisted,
                    "Cancelled" => AttendanceStatus::Cancelled,
                    _ => {
                        return Err(AppError::Internal(format!(
                            "Invalid attendance status: {}",
                            status
                        )))
                    }
                };
                Ok(Some(attendance_status))
            }
            None => Ok(None),
        }
    }

    async fn max_occurrence_index_for_series(&self, series_id: Uuid) -> Result<Option<i32>> {
        let max: Option<i32> =
            sqlx::query_scalar("SELECT MAX(occurrence_index) FROM events WHERE series_id = ?")
                .bind(series_id.to_string())
                .fetch_one(&self.pool)
                .await
                .map_err(AppError::Database)?;
        Ok(max)
    }

    async fn find_by_series_and_index(
        &self,
        series_id: Uuid,
        occurrence_index: i32,
    ) -> Result<Option<Event>> {
        let row = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT id, title, description, event_type, event_type_id, visibility,
                   start_time, end_time, location, max_attendees, rsvp_required,
                   image_url, created_by, created_at, updated_at,
                   series_id, occurrence_index
            FROM events
            WHERE series_id = ? AND occurrence_index = ?
            "#,
        )
        .bind(series_id.to_string())
        .bind(occurrence_index)
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        match row {
            Some(r) => Ok(Some(Self::row_to_event(r)?)),
            None => Ok(None),
        }
    }

    async fn delete_series_occurrences_after(
        &self,
        series_id: Uuid,
        after: DateTime<Utc>,
    ) -> Result<u64> {
        let result = sqlx::query("DELETE FROM events WHERE series_id = ? AND start_time > ?")
            .bind(series_id.to_string())
            .bind(after.naive_utc())
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;
        Ok(result.rows_affected())
    }

    async fn update_series_occurrences_from(
        &self,
        series_id: Uuid,
        from: DateTime<Utc>,
        template: &crate::domain::Event,
    ) -> Result<u64> {
        // Apply the "edit this and all future" subset. Per-occurrence
        // start_time/end_time/image_url stay intact — those are
        // properties of the specific occurrence, not the series.
        let event_type_str = Self::event_type_to_str(&template.event_type);
        let visibility_str = Self::visibility_to_str(&template.visibility);
        let event_type_id_str = template.event_type_id.map(|id| id.to_string());
        let rsvp_int = if template.rsvp_required { 1i32 } else { 0i32 };

        let result = sqlx::query(
            r#"
            UPDATE events
            SET title = ?,
                description = ?,
                event_type = ?,
                event_type_id = ?,
                visibility = ?,
                location = ?,
                max_attendees = ?,
                rsvp_required = ?,
                updated_at = ?
            WHERE series_id = ? AND start_time >= ?
            "#,
        )
        .bind(&template.title)
        .bind(&template.description)
        .bind(event_type_str)
        .bind(&event_type_id_str)
        .bind(visibility_str)
        .bind(&template.location)
        .bind(template.max_attendees)
        .bind(rsvp_int)
        .bind(Utc::now().naive_utc())
        .bind(series_id.to_string())
        .bind(from.naive_utc())
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(result.rows_affected())
    }

    async fn list_pending_reminders(
        &self,
        now: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> Result<Vec<EventReminderRow>> {
        let rows: Vec<(
            String,
            String,
            NaiveDateTime,
            Option<String>,
            String,
            String,
            String,
        )> = sqlx::query_as(
            r#"
                SELECT e.id, e.title, e.start_time, e.location,
                       m.id, m.email, m.full_name
                FROM event_attendance ea
                JOIN events e ON e.id = ea.event_id
                JOIN members m ON m.id = ea.member_id
                WHERE ea.status = 'Registered'
                  AND ea.reminder_sent_at IS NULL
                  AND e.start_time > ?
                  AND e.start_time <= ?
                "#,
        )
        .bind(now.naive_utc())
        .bind(until.naive_utc())
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        rows.into_iter()
            .map(|(eid, title, start, location, mid, email, full_name)| {
                Ok(EventReminderRow {
                    event_id: Uuid::parse_str(&eid)
                        .map_err(|e| AppError::Internal(e.to_string()))?,
                    event_title: title,
                    event_start: DateTime::from_naive_utc_and_offset(start, Utc),
                    event_location: location,
                    member_id: Uuid::parse_str(&mid)
                        .map_err(|e| AppError::Internal(e.to_string()))?,
                    member_email: email,
                    member_full_name: full_name,
                })
            })
            .collect()
    }

    async fn mark_reminder_sent(&self, event_id: Uuid, member_id: Uuid) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE event_attendance
            SET reminder_sent_at = CURRENT_TIMESTAMP
            WHERE event_id = ? AND member_id = ? AND reminder_sent_at IS NULL
            "#,
        )
        .bind(event_id.to_string())
        .bind(member_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(result.rows_affected() == 1)
    }
}
