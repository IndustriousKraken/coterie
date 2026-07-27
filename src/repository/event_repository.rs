use async_trait::async_trait;
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use chrono_tz::Tz;
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

use crate::{
    domain::{
        wall_clock_to_utc, AttendanceStatus, Attendee, Event, EventType, EventVisibility,
        PaymentMethod, PaymentStatus,
    },
    error::{AppError, Result},
};

/// Identity predicate for one attendance row, NULL-safe on both sides.
/// `IS` rather than `=` so a bound NULL matches a NULL column — that is
/// what lets one statement serve a member seat (`guest_email` NULL) and a
/// guest seat (`member_id` NULL) instead of forking every seat query in
/// two. Binds are (member_id, guest_email) in that order.
const ATTENDEE_MATCH: &str = "member_id IS ? AND guest_email IS ?";

/// The held-seat predicate, shared by `count_held_seats` and the
/// capacity guard inside `claim_seat` so the two can never disagree
/// about what "full" means. Expects the attendance row aliased `a` and
/// a LEFT JOIN of `payments` aliased `p`.
///
/// A `PendingPayment` row holds its seat while its payment is still
/// `Pending` — and also while `payment_id` is still NULL, which is the
/// instant between claiming the seat and creating the Checkout session.
/// Without that second case two members racing for the last seat both
/// see it free, because neither has linked a payment yet.
///
/// ponytail: a row that never gets linked (the process dies between the
/// claim and the session) therefore holds its seat until an admin uses
/// the roster's release control — the same bounded, manually-fixable
/// ceiling as a webhook that never arrives. A background sweeper is
/// speculative until an org actually reports a stuck seat.
const HELD_SEAT_PREDICATE: &str = "(a.status = 'Registered' \
     OR (a.status = 'PendingPayment' AND (a.payment_id IS NULL OR p.status = 'Pending')))";

/// One candidate row for the event-reminder runner — a flat join of
/// the attendee, event, and member rows that the runner needs to
/// render and send a reminder. Kept narrow on purpose: only the
/// fields the template + claim step actually touch.
#[derive(Debug, Clone)]
pub struct EventReminderRow {
    pub event_id: Uuid,
    pub event_title: String,
    /// The event's local wall-clock (naive component of the container),
    /// paired with `timezone`. Derive the true instant via `start_utc`;
    /// do NOT treat this as UTC. Same model as `Event::start_time`.
    pub event_start: DateTime<Utc>,
    /// IANA zone the wall-clock is in, frozen on the event row.
    pub timezone: String,
    pub event_location: Option<String>,
    pub member_id: Uuid,
    pub member_email: String,
    pub member_full_name: String,
}

impl EventReminderRow {
    /// The event's IANA zone, falling back to UTC. Mirrors `Event::tz`.
    pub fn tz(&self) -> Tz {
        self.timezone.parse().unwrap_or(Tz::UTC)
    }

    /// The true UTC instant, derived from the stored (wall-clock, zone).
    /// The reminder window filter and the email render both use this so
    /// a non-UTC org's reminders fire at the real instant rather than
    /// the wall-clock mislabeled as UTC. Same as `Event::start_utc`.
    pub fn start_utc(&self) -> DateTime<Utc> {
        wall_clock_to_utc(self.event_start.naive_utc(), self.tz())
    }
}

/// One line of the admin roster: who is on the event, what state their
/// seat is in, and what state the money is in. Payment fields are `None`
/// for a free RSVP (no payment row exists) — that absence IS the answer,
/// so it isn't papered over with a default.
#[derive(Debug, Clone)]
pub struct RosterEntry {
    /// Who holds the seat — a member, or a guest who registered through
    /// the public page. Drives the roster's per-row actions, which are
    /// keyed on member id for one and guest email for the other.
    pub attendee: Attendee,
    /// Display name: the member's full name, or the guest's supplied name.
    pub name: String,
    /// Display email: the member's, or the guest's supplied address.
    pub email: String,
    pub status: AttendanceStatus,
    pub payment_id: Option<Uuid>,
    pub payment_status: Option<PaymentStatus>,
    pub payment_method: Option<PaymentMethod>,
    pub amount_cents: Option<i64>,
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
    /// Seat `attendee` as `Registered` outright, capacity advisory —
    /// today's free-RSVP upsert, and the admin's at-the-door/comp path.
    /// Works for a guest on the same terms: the identity columns differ,
    /// the seat does not.
    async fn register_attendance(&self, event_id: Uuid, attendee: &Attendee) -> Result<()>;
    async fn cancel_attendance(&self, event_id: Uuid, attendee: &Attendee) -> Result<()>;
    async fn get_attendee_count(&self, event_id: Uuid) -> Result<i64>;
    /// The seat status for one identity on one event, or `None` when they
    /// hold no seat.
    async fn attendance_status(
        &self,
        event_id: Uuid,
        attendee: &Attendee,
    ) -> Result<Option<AttendanceStatus>>;

    // ---- Paid-event seats ---------------------------------------------

    /// Seats currently held: `Registered` rows, plus `PendingPayment`
    /// rows whose linked payment is still `Pending`. A pending seat
    /// whose payment left `Pending` (expired checkout, failed intent)
    /// stops holding capacity by virtue of that flip — nothing has to
    /// delete the row, which is what lets the existing
    /// `checkout.session.expired` handler free a seat with no new code.
    async fn count_held_seats(&self, event_id: Uuid) -> Result<i64>;

    /// Atomically claim a seat as `PendingPayment`, rejecting with
    /// `BadRequest` when the event is already full. The count and the
    /// insert are ONE statement — a count outside the write is the race
    /// this method exists to prevent, because a lost race for a paid
    /// seat is a refund incident rather than a rejected click.
    ///
    /// `max_attendees` of `None` means uncapped.
    ///
    /// Guest and member seats compete for the same capacity because they
    /// are rows in the same table and the count is row-based.
    async fn claim_seat(
        &self,
        event_id: Uuid,
        attendee: &Attendee,
        max_attendees: Option<i32>,
    ) -> Result<()>;

    /// Point a claimed seat at the payment that will pay for it.
    async fn link_payment(
        &self,
        event_id: Uuid,
        attendee: &Attendee,
        payment_id: Uuid,
    ) -> Result<()>;

    /// Promote the seat linked to `payment_id` from `PendingPayment` to
    /// `Registered`. Conditional on the row still being pending, so a
    /// late webhook can't resurrect a seat that was already cancelled.
    /// Returns true when a row moved.
    async fn confirm_seat(&self, payment_id: Uuid) -> Result<bool>;

    /// Drop a `PendingPayment` claim entirely — the rollback for a
    /// checkout session that could not be created, and the admin's
    /// release-a-stuck-seat control. Never touches a confirmed seat.
    async fn release_seat(&self, event_id: Uuid, attendee: &Attendee) -> Result<()>;

    /// Cancel the seat linked to `payment_id` — the refund path. A
    /// member does not keep a seat for an event whose fee was returned.
    async fn cancel_seat_for_payment(&self, payment_id: Uuid) -> Result<()>;

    /// Attendees of an event with their attendance status and the state
    /// of any linked payment, for the admin roster.
    async fn roster(&self, event_id: Uuid) -> Result<Vec<RosterEntry>>;

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
    /// Every materialized occurrence of a series, `start_time` ascending.
    /// Series-pass enrollment reads this to decide which occurrences are
    /// still in the future (the tz-correct test needs `Event::start_utc`,
    /// which SQL can't compute), and the class roster + delete paths
    /// reuse it.
    async fn list_series_occurrences(&self, series_id: Uuid) -> Result<Vec<Event>>;
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
    timezone: String,
    location: Option<String>,
    max_attendees: Option<i32>,
    rsvp_required: i32,
    member_price_cents: i64,
    guest_price_cents: i64,
    guest_registration_enabled: i32,
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
            timezone: row.timezone,
            location: row.location,
            max_attendees: row.max_attendees,
            rsvp_required: row.rsvp_required != 0,
            member_price_cents: row.member_price_cents,
            guest_price_cents: row.guest_price_cents,
            guest_registration_enabled: row.guest_registration_enabled != 0,
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

    fn parse_attendance_status(s: &str) -> Result<AttendanceStatus> {
        match s {
            "Registered" => Ok(AttendanceStatus::Registered),
            "Waitlisted" => Ok(AttendanceStatus::Waitlisted),
            "Cancelled" => Ok(AttendanceStatus::Cancelled),
            "PendingPayment" => Ok(AttendanceStatus::PendingPayment),
            _ => Err(AppError::Internal(format!(
                "Invalid attendance status: {}",
                s
            ))),
        }
    }

    /// Joined payment columns are read leniently: an unrecognized value
    /// on the roster costs a display detail, not the whole page.
    fn parse_payment_status(s: &str) -> Option<PaymentStatus> {
        match s {
            "Pending" => Some(PaymentStatus::Pending),
            "Completed" => Some(PaymentStatus::Completed),
            "Failed" => Some(PaymentStatus::Failed),
            "Refunded" => Some(PaymentStatus::Refunded),
            _ => None,
        }
    }

    fn parse_payment_method(s: &str) -> Option<PaymentMethod> {
        match s {
            "Stripe" => Some(PaymentMethod::Stripe),
            "Manual" => Some(PaymentMethod::Manual),
            "Waived" => Some(PaymentMethod::Waived),
            _ => None,
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
                start_time, end_time, timezone, location, max_attendees, rsvp_required,
                member_price_cents, guest_price_cents, guest_registration_enabled,
                image_url, created_by, created_at, updated_at,
                series_id, occurrence_index
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
        .bind(&event.timezone)
        .bind(&event.location)
        .bind(max_attendees_int)
        .bind(rsvp_required_int)
        .bind(event.member_price_cents)
        .bind(event.guest_price_cents)
        .bind(if event.guest_registration_enabled {
            1i32
        } else {
            0i32
        })
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
                   start_time, end_time, timezone, location, max_attendees, rsvp_required,
                   member_price_cents, guest_price_cents, guest_registration_enabled,
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
                   start_time, end_time, timezone, location, max_attendees, rsvp_required,
                   member_price_cents, guest_price_cents, guest_registration_enabled,
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
        // `start_time` is a naive wall-clock, so comparing it to UTC `now`
        // in SQL drops non-UTC-org events by the org's offset (a 7 PM EDT
        // event would fall out of "upcoming" at 3 PM). Widen the SQL bound
        // by the widest IANA offset (~14h) as a coarse pre-filter, then do
        // the exact `start_utc() > now` test in Rust and apply the limit
        // there — the same pattern as `list_pending_reminders`.
        let now = Utc::now();
        let margin = Duration::hours(15);

        let rows = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT id, title, description, event_type, event_type_id, visibility,
                   start_time, end_time, timezone, location, max_attendees, rsvp_required,
                   member_price_cents, guest_price_cents, guest_registration_enabled,
                   image_url, created_by, created_at, updated_at,
                   series_id, occurrence_index
            FROM events
            WHERE start_time > ?
            ORDER BY start_time ASC
            "#,
        )
        .bind((now - margin).naive_utc())
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        let mut events: Vec<Event> = rows
            .into_iter()
            .map(Self::row_to_event)
            .collect::<Result<Vec<_>>>()?;
        // Exact test + ordering on the derived instant, then the limit.
        events.retain(|e| e.start_utc() > now);
        events.sort_by(|a, b| a.start_utc().cmp(&b.start_utc()));
        events.truncate(limit as usize);
        Ok(events)
    }

    async fn list_public(&self) -> Result<Vec<Event>> {
        let visibility_str = Self::visibility_to_str(&EventVisibility::Public);

        let rows = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT id, title, description, event_type, event_type_id, visibility,
                   start_time, end_time, timezone, location, max_attendees, rsvp_required,
                   member_price_cents, guest_price_cents, guest_registration_enabled,
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
                   start_time, end_time, timezone, location, max_attendees, rsvp_required,
                   member_price_cents, guest_price_cents, guest_registration_enabled,
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
        // Count on the derived UTC instant, not the raw wall-clock (which
        // would mis-count by the org's offset near evening events). SQLite
        // can't do the tz math, so fetch the widened candidate set and
        // count those still upcoming by their true instant — same pattern
        // as `list_upcoming`.
        let visibility_str = Self::visibility_to_str(&EventVisibility::MembersOnly);
        let now = Utc::now();
        let margin = Duration::hours(15);

        let rows = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT id, title, description, event_type, event_type_id, visibility,
                   start_time, end_time, timezone, location, max_attendees, rsvp_required,
                   member_price_cents, guest_price_cents, guest_registration_enabled,
                   image_url, created_by, created_at, updated_at,
                   series_id, occurrence_index
            FROM events
            WHERE visibility = ? AND start_time > ?
            "#,
        )
        .bind(visibility_str)
        .bind((now - margin).naive_utc())
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        let count = rows
            .into_iter()
            .map(Self::row_to_event)
            .collect::<Result<Vec<_>>>()?
            .iter()
            .filter(|e| e.start_utc() > now)
            .count();
        Ok(count as i64)
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
                rsvp_required = ?, member_price_cents = ?, guest_price_cents = ?,
                guest_registration_enabled = ?, image_url = ?, updated_at = ?
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
        .bind(event.member_price_cents)
        .bind(event.guest_price_cents)
        .bind(if event.guest_registration_enabled {
            1i32
        } else {
            0i32
        })
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

    async fn register_attendance(&self, event_id: Uuid, attendee: &Attendee) -> Result<()> {
        // The conflict target is omitted so the one statement upserts on
        // whichever identity constraint the row collides with —
        // `(event_id, member_id)` for a member, `(event_id, guest_email)`
        // for a guest.
        sqlx::query(
            r#"
            INSERT INTO event_attendance
                (id, event_id, member_id, guest_name, guest_email, status, registered_at)
            VALUES (?, ?, ?, ?, ?, 'Registered', CURRENT_TIMESTAMP)
            ON CONFLICT
            DO UPDATE SET status = 'Registered', registered_at = CURRENT_TIMESTAMP
            "#,
        )
        .bind(Uuid::new_v4().to_string())
        .bind(event_id.to_string())
        .bind(attendee.member_id().map(|id| id.to_string()))
        .bind(attendee.guest_name())
        .bind(attendee.guest_email())
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;

        Ok(())
    }

    async fn cancel_attendance(&self, event_id: Uuid, attendee: &Attendee) -> Result<()> {
        let sql = format!(
            "UPDATE event_attendance SET status = 'Cancelled' \
             WHERE event_id = ? AND {ATTENDEE_MATCH}",
        );
        sqlx::query(&sql)
            .bind(event_id.to_string())
            .bind(attendee.member_id().map(|id| id.to_string()))
            .bind(attendee.guest_email())
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

    async fn attendance_status(
        &self,
        event_id: Uuid,
        attendee: &Attendee,
    ) -> Result<Option<AttendanceStatus>> {
        let sql =
            format!("SELECT status FROM event_attendance WHERE event_id = ? AND {ATTENDEE_MATCH}",);
        let row: Option<(String,)> = sqlx::query_as(&sql)
            .bind(event_id.to_string())
            .bind(attendee.member_id().map(|id| id.to_string()))
            .bind(attendee.guest_email())
            .fetch_optional(&self.pool)
            .await
            .map_err(AppError::Database)?;

        match row {
            Some((status,)) => Ok(Some(Self::parse_attendance_status(&status)?)),
            None => Ok(None),
        }
    }

    async fn count_held_seats(&self, event_id: Uuid) -> Result<i64> {
        let sql = format!(
            "SELECT COUNT(*) FROM event_attendance a \
             LEFT JOIN payments p ON p.id = a.payment_id \
             WHERE a.event_id = ? AND {HELD_SEAT_PREDICATE}",
        );
        let row: (i64,) = sqlx::query_as(&sql)
            .bind(event_id.to_string())
            .fetch_one(&self.pool)
            .await
            .map_err(AppError::Database)?;
        Ok(row.0)
    }

    async fn claim_seat(
        &self,
        event_id: Uuid,
        attendee: &Attendee,
        max_attendees: Option<i32>,
    ) -> Result<()> {
        // One statement, so SQLite holds the write lock across the
        // count AND the insert: two members racing for the last seat
        // cannot both see "one left". An explicit BEGIN/COUNT/INSERT
        // transaction would be a deferred read that upgrades to a write
        // — the exact window this avoids.
        //
        // The `ON CONFLICT` arm re-claims a row the attendee previously
        // cancelled (or abandoned); the capacity guard in the SELECT
        // still applies, since neither of those states holds a seat. The
        // conflict target is omitted so it fires on whichever identity
        // constraint collides — member or guest.
        let cap = max_attendees.map(i64::from).unwrap_or(i64::MAX);
        let sql = format!(
            "INSERT INTO event_attendance \
                 (id, event_id, member_id, guest_name, guest_email, status, registered_at, payment_id) \
             SELECT ?1, ?2, ?3, ?4, ?5, 'PendingPayment', CURRENT_TIMESTAMP, NULL \
             WHERE (SELECT COUNT(*) FROM event_attendance a \
                    LEFT JOIN payments p ON p.id = a.payment_id \
                    WHERE a.event_id = ?2 AND {HELD_SEAT_PREDICATE}) < ?6 \
             ON CONFLICT DO UPDATE \
             SET status = 'PendingPayment', registered_at = CURRENT_TIMESTAMP, payment_id = NULL",
        );
        let res = sqlx::query(&sql)
            .bind(Uuid::new_v4().to_string())
            .bind(event_id.to_string())
            .bind(attendee.member_id().map(|id| id.to_string()))
            .bind(attendee.guest_name())
            .bind(attendee.guest_email())
            .bind(cap)
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;

        if res.rows_affected() == 0 {
            return Err(AppError::BadRequest(
                "This event is full — no seats are available".to_string(),
            ));
        }
        Ok(())
    }

    async fn link_payment(
        &self,
        event_id: Uuid,
        attendee: &Attendee,
        payment_id: Uuid,
    ) -> Result<()> {
        let sql = format!(
            "UPDATE event_attendance SET payment_id = ? \
             WHERE event_id = ? AND {ATTENDEE_MATCH}",
        );
        sqlx::query(&sql)
            .bind(payment_id.to_string())
            .bind(event_id.to_string())
            .bind(attendee.member_id().map(|id| id.to_string()))
            .bind(attendee.guest_email())
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;
        Ok(())
    }

    async fn confirm_seat(&self, payment_id: Uuid) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE event_attendance SET status = 'Registered' \
             WHERE payment_id = ? AND status = 'PendingPayment'",
        )
        .bind(payment_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(res.rows_affected() > 0)
    }

    async fn release_seat(&self, event_id: Uuid, attendee: &Attendee) -> Result<()> {
        let sql = format!(
            "DELETE FROM event_attendance \
             WHERE event_id = ? AND {ATTENDEE_MATCH} AND status = 'PendingPayment'",
        );
        sqlx::query(&sql)
            .bind(event_id.to_string())
            .bind(attendee.member_id().map(|id| id.to_string()))
            .bind(attendee.guest_email())
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;
        Ok(())
    }

    async fn cancel_seat_for_payment(&self, payment_id: Uuid) -> Result<()> {
        sqlx::query("UPDATE event_attendance SET status = 'Cancelled' WHERE payment_id = ?")
            .bind(payment_id.to_string())
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;
        Ok(())
    }

    async fn roster(&self, event_id: Uuid) -> Result<Vec<RosterEntry>> {
        // LEFT JOIN, not JOIN: a guest row has no member to join to, and
        // an inner join would silently hide every guest from the roster.
        #[allow(clippy::type_complexity)]
        let rows: Vec<(
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<i64>,
        )> = sqlx::query_as(
            r#"
            SELECT a.member_id, m.full_name, m.email, a.guest_name, a.guest_email, a.status,
                   a.payment_id, p.status, p.payment_method, p.amount_cents
            FROM event_attendance a
            LEFT JOIN members m ON m.id = a.member_id
            LEFT JOIN payments p ON p.id = a.payment_id
            WHERE a.event_id = ?
            ORDER BY a.registered_at ASC
            "#,
        )
        .bind(event_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        rows.into_iter()
            .map(
                |(
                    mid,
                    m_name,
                    m_email,
                    guest_name,
                    guest_email,
                    status,
                    pay_id,
                    pay_status,
                    pay_method,
                    amount,
                )| {
                    // The DB CHECK guarantees exactly one identity, so
                    // the member branch is taken iff member_id is set.
                    let (attendee, name, email) = match mid {
                        Some(mid) => (
                            Attendee::Member(
                                Uuid::parse_str(&mid)
                                    .map_err(|e| AppError::Internal(e.to_string()))?,
                            ),
                            m_name.unwrap_or_default(),
                            m_email.unwrap_or_default(),
                        ),
                        None => {
                            let email = guest_email.unwrap_or_default();
                            let name = guest_name.unwrap_or_default();
                            (
                                Attendee::Guest {
                                    name: name.clone(),
                                    email: email.clone(),
                                },
                                name,
                                email,
                            )
                        }
                    };
                    Ok(RosterEntry {
                        attendee,
                        name,
                        email,
                        status: Self::parse_attendance_status(&status)?,
                        payment_id: pay_id.as_deref().and_then(|s| Uuid::parse_str(s).ok()),
                        payment_status: pay_status.as_deref().and_then(Self::parse_payment_status),
                        payment_method: pay_method.as_deref().and_then(Self::parse_payment_method),
                        amount_cents: amount,
                    })
                },
            )
            .collect()
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

    async fn list_series_occurrences(&self, series_id: Uuid) -> Result<Vec<Event>> {
        let rows = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT id, title, description, event_type, event_type_id, visibility,
                   start_time, end_time, timezone, location, max_attendees, rsvp_required,
                   member_price_cents, guest_price_cents, guest_registration_enabled,
                   image_url, created_by, created_at, updated_at,
                   series_id, occurrence_index
            FROM events
            WHERE series_id = ?
            ORDER BY start_time ASC
            "#,
        )
        .bind(series_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        rows.into_iter().map(Self::row_to_event).collect()
    }

    async fn find_by_series_and_index(
        &self,
        series_id: Uuid,
        occurrence_index: i32,
    ) -> Result<Option<Event>> {
        let row = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT id, title, description, event_type, event_type_id, visibility,
                   start_time, end_time, timezone, location, max_attendees, rsvp_required,
                   member_price_cents, guest_price_cents, guest_registration_enabled,
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
                member_price_cents = ?,
                guest_price_cents = ?,
                guest_registration_enabled = ?,
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
        .bind(template.member_price_cents)
        .bind(template.guest_price_cents)
        .bind(if template.guest_registration_enabled {
            1i32
        } else {
            0i32
        })
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
        // `e.start_time` is a naive wall-clock, so it can't be compared
        // to the UTC `now`/`until` directly (that's the org-offset bug
        // this change fixes elsewhere). The SQL bound is a coarse
        // pre-filter widened by the widest possible IANA offset (~14h);
        // the exact `(now, until]` test on the derived UTC happens in
        // Rust below, where the tz database is available.
        let margin = Duration::hours(15);
        let rows: Vec<(
            String,
            String,
            NaiveDateTime,
            String,
            Option<String>,
            String,
            String,
            String,
        )> = sqlx::query_as(
            r#"
                SELECT e.id, e.title, e.start_time, e.timezone, e.location,
                       m.id, m.email, m.full_name
                -- Inner join on members: event reminders are a member
                -- benefit rendered from the member row, so guest seats
                -- are not candidates. Their confirmation email at
                -- registration time is their one artifact.
                FROM event_attendance ea
                JOIN events e ON e.id = ea.event_id
                JOIN members m ON m.id = ea.member_id
                WHERE ea.status = 'Registered'
                  AND ea.reminder_sent_at IS NULL
                  AND e.start_time > ?
                  AND e.start_time <= ?
                "#,
        )
        .bind((now - margin).naive_utc())
        .bind((until + margin).naive_utc())
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        let mut out: Vec<EventReminderRow> = rows
            .into_iter()
            .map(
                |(eid, title, start, timezone, location, mid, email, full_name)| {
                    Ok(EventReminderRow {
                        event_id: Uuid::parse_str(&eid)
                            .map_err(|e| AppError::Internal(e.to_string()))?,
                        event_title: title,
                        event_start: DateTime::from_naive_utc_and_offset(start, Utc),
                        timezone,
                        event_location: location,
                        member_id: Uuid::parse_str(&mid)
                            .map_err(|e| AppError::Internal(e.to_string()))?,
                        member_email: email,
                        member_full_name: full_name,
                    })
                },
            )
            .collect::<Result<_>>()?;

        // Exact window test on the derived instant, not the wall-clock.
        out.retain(|row| {
            let start_utc = row.start_utc();
            start_utc > now && start_utc <= until
        });
        Ok(out)
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
