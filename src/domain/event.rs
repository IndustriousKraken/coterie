use chrono::{DateTime, Duration, LocalResult, NaiveDateTime, TimeZone, Utc};
use chrono_tz::{OffsetName, Tz};
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
    /// The event's **local wall-clock** time — NOT a real UTC instant.
    /// Its naive component (year/month/day hour:min) is what the
    /// organizer typed; it is paired with [`Event::timezone`] to derive
    /// the actual instant via [`Event::start_utc`]. It is kept in a
    /// `DateTime<Utc>` only as a naive container (the DB column is a
    /// naive DATETIME) so the wall-clock survives a government rule
    /// change unshifted. Render this directly for the admin surface;
    /// derive UTC for public/iCal output.
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    /// IANA zone name (e.g. `America/New_York`) the wall-clock is
    /// understood in. Frozen at creation from `org.timezone`.
    pub timezone: String,
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

impl Event {
    /// The event's IANA zone, falling back to UTC when the stored name
    /// is empty or unrecognized. The setting validator rejects bad zone
    /// names on write, so the fallback is purely defensive.
    pub fn tz(&self) -> Tz {
        self.timezone.parse().unwrap_or(Tz::UTC)
    }

    /// Derive the true UTC instant from the stored (wall-clock, zone)
    /// using the current tz database. Recomputed on every read, so a
    /// later change to the zone's rules is picked up automatically.
    pub fn start_utc(&self) -> DateTime<Utc> {
        wall_clock_to_utc(self.start_time.naive_utc(), self.tz())
    }

    /// The event's zone abbreviation at its wall-clock (e.g. `EDT` /
    /// `EST` / `UTC`), for labeling member-facing times so a remote
    /// viewer isn't misled about which local time the event is in.
    /// Mirrors the reminder-email rendering.
    pub fn zone_abbr(&self) -> String {
        self.start_utc()
            .with_timezone(&self.tz())
            .offset()
            .abbreviation()
            .unwrap_or("UTC")
            .to_string()
    }

    /// UTC instant for the end time, if any. See [`Event::start_utc`].
    pub fn end_utc(&self) -> Option<DateTime<Utc>> {
        self.end_time
            .map(|e| wall_clock_to_utc(e.naive_utc(), self.tz()))
    }
}

/// Resolve a naive local wall-clock in `tz` to a UTC instant, handling
/// the two DST edge cases with a defined rule instead of panicking:
///   - **overlap** (fall-back hour, the wall-clock happens twice): pick
///     the earliest (pre-transition) instant.
///   - **gap** (spring-forward hour, the wall-clock never happens): shift
///     the wall-clock forward past the gap, so it lands just after the
///     transition rather than being rejected.
pub fn wall_clock_to_utc(naive: NaiveDateTime, tz: Tz) -> DateTime<Utc> {
    match tz.from_local_datetime(&naive) {
        LocalResult::Single(dt) => dt.with_timezone(&Utc),
        LocalResult::Ambiguous(earliest, _) => earliest.with_timezone(&Utc),
        LocalResult::None => match tz.from_local_datetime(&(naive + Duration::hours(1))) {
            LocalResult::Single(dt) => dt.with_timezone(&Utc),
            LocalResult::Ambiguous(dt, _) => dt.with_timezone(&Utc),
            // No realistic zone has a multi-hour gap; treat the naive
            // value as already-UTC as a last resort.
            LocalResult::None => DateTime::from_naive_utc_and_offset(naive, Utc),
        },
    }
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

#[cfg(test)]
mod tz_tests {
    use super::*;

    fn wall(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        chrono::NaiveDate::from_ymd_opt(y, mo, d)
            .unwrap()
            .and_hms_opt(h, mi, 0)
            .unwrap()
    }

    fn event_at(naive: NaiveDateTime, zone: &str) -> Event {
        Event {
            id: Uuid::new_v4(),
            title: "T".into(),
            description: String::new(),
            event_type: EventType::Meeting,
            event_type_id: None,
            visibility: EventVisibility::Public,
            // The stored wall-clock lives in the naive component of a
            // DateTime<Utc> container (mirrors the DB naive column).
            start_time: DateTime::from_naive_utc_and_offset(naive, Utc),
            end_time: Some(DateTime::from_naive_utc_and_offset(
                naive + Duration::hours(1),
                Utc,
            )),
            timezone: zone.into(),
            location: None,
            max_attendees: None,
            rsvp_required: false,
            image_url: None,
            created_by: Uuid::new_v4(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            series_id: None,
            occurrence_index: None,
        }
    }

    // zone_abbr labels the wall-clock with the event's real zone (EDT
    // in July, EST in January), so member-facing renderers don't
    // mislabel a New York event as "UTC".
    #[test]
    fn zone_abbr_reflects_dst_and_zone() {
        assert_eq!(
            event_at(wall(2026, 7, 23, 19, 0), "America/New_York").zone_abbr(),
            "EDT"
        );
        assert_eq!(
            event_at(wall(2026, 1, 23, 19, 0), "America/New_York").zone_abbr(),
            "EST"
        );
        assert_eq!(event_at(wall(2026, 7, 23, 19, 0), "UTC").zone_abbr(), "UTC");
    }

    // 7.1 derivation: 7 PM Eastern in July (EDT, UTC-4) → 23:00Z.
    #[test]
    fn start_utc_derives_july_eastern() {
        let e = event_at(wall(2026, 7, 23, 19, 0), "America/New_York");
        assert_eq!(
            e.start_utc().to_rfc3339(),
            "2026-07-23T23:00:00+00:00",
            "7 PM EDT should derive to 23:00Z"
        );
        assert_eq!(
            e.end_utc().unwrap().to_rfc3339(),
            "2026-07-24T00:00:00+00:00"
        );
        // The stored wall-clock (admin surface) is untouched by derivation.
        assert_eq!(
            e.start_time.format("%Y-%m-%dT%H:%M").to_string(),
            "2026-07-23T19:00"
        );
    }

    // 7.2 rule resilience: the SAME stored wall-clock yields different
    // instants by date because the tz database's offset is applied at
    // read time, not frozen. July = EDT (UTC-4), January = EST (UTC-5).
    #[test]
    fn same_wallclock_different_instant_across_seasons() {
        let july = event_at(wall(2026, 7, 23, 19, 0), "America/New_York");
        let jan = event_at(wall(2026, 1, 23, 19, 0), "America/New_York");
        assert_eq!(july.start_utc().to_rfc3339(), "2026-07-23T23:00:00+00:00");
        assert_eq!(jan.start_utc().to_rfc3339(), "2026-01-24T00:00:00+00:00");
    }

    // UTC zone is a no-op: the derived instant equals the stored wall-clock.
    #[test]
    fn utc_zone_is_identity() {
        let e = event_at(wall(2026, 7, 23, 19, 0), "UTC");
        assert_eq!(e.start_utc().to_rfc3339(), "2026-07-23T19:00:00+00:00");
    }

    // An empty / unknown zone name falls back to UTC rather than panicking.
    #[test]
    fn unknown_zone_falls_back_to_utc() {
        let e = event_at(wall(2026, 7, 23, 19, 0), "Not/AZone");
        assert_eq!(e.tz(), Tz::UTC);
        assert_eq!(e.start_utc().to_rfc3339(), "2026-07-23T19:00:00+00:00");
    }

    // DST gap (spring-forward): 02:30 on 2026-03-08 does not exist in
    // America/New_York. The defined rule shifts forward past the gap
    // instead of panicking — the result is a valid instant.
    #[test]
    fn dst_gap_resolves_without_panic() {
        let utc = wall_clock_to_utc(wall(2026, 3, 8, 2, 30), Tz::America__New_York);
        // 03:30 EDT = 07:30Z (the "would-be" 02:30 shifted forward).
        assert_eq!(utc.to_rfc3339(), "2026-03-08T07:30:00+00:00");
    }

    // DST overlap (fall-back): 01:30 on 2026-11-01 happens twice in
    // America/New_York. The defined rule picks the earliest (EDT) instant.
    #[test]
    fn dst_overlap_picks_earliest() {
        let utc = wall_clock_to_utc(wall(2026, 11, 1, 1, 30), Tz::America__New_York);
        // Earliest = still-EDT (UTC-4) → 05:30Z (vs the later EST 06:30Z).
        assert_eq!(utc.to_rfc3339(), "2026-11-01T05:30:00+00:00");
    }
}
