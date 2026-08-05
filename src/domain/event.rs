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
    /// What a member pays to attend, in cents. `0` means free — a
    /// *known* price, not an unknown one, which is why this is not an
    /// `Option`. See [`Event::is_paid_for_members`].
    pub member_price_cents: i64,
    /// What a non-member pays to attend, in cents. `0` means free, on
    /// the same terms as the member price. Says nothing about *whether*
    /// non-members may register — that's
    /// [`Event::guest_registration_enabled`].
    pub guest_price_cents: i64,
    /// Whether non-members may register through the public page at all.
    /// Kept separate from the price so "the public attends free" and
    /// "the public may not attend" are different states. See
    /// [`Event::publicly_registerable`].
    pub guest_registration_enabled: bool,
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
    /// True when attending costs a member money. The single place the
    /// free-vs-paid rule lives, so templates and services don't
    /// re-derive `> 0` and drift on the boundary.
    pub fn is_paid_for_members(&self) -> bool {
        self.member_price_cents > 0
    }

    /// True when attending costs a non-member money. Mirrors
    /// [`Event::is_paid_for_members`]; `false` means the public
    /// registration is free, NOT that it is closed.
    pub fn is_paid_for_guests(&self) -> bool {
        self.guest_price_cents > 0
    }

    /// Whether the public registration page and endpoint serve this
    /// event: it is `Public` AND the org opened the door to non-members.
    ///
    /// The price is deliberately NOT part of this test — a free workshop
    /// with twenty seats is registerable, and a $0 price must not close
    /// a public door any more than a `NULL` should have meant "free".
    /// One home for the rule so the 404 decision isn't re-derived (and
    /// re-broken) per call site.
    pub fn publicly_registerable(&self) -> bool {
        self.visibility == EventVisibility::Public && self.guest_registration_enabled
    }

    /// Strip everything a `MembersOnly` event does not disclose to an
    /// anonymous caller, leaving exactly what `/public/events` returns
    /// for it: a placeholder title and description, no location, no
    /// image, and no public registration door.
    ///
    /// The one implementation of "what may leave" for a members-only
    /// event. Both the public feed and the integration dispatch call it,
    /// so the two cannot drift into disagreeing about what is disclosed
    /// — and a drift there would be silent.
    pub fn sanitize_members_only(&mut self) {
        self.title = "Members-Only Event".to_string();
        self.description =
            "This event is for members only. Log in to the portal to see details.".to_string();
        self.location = None;
        self.image_url = None;
        // Cleared alongside the rest so a members-only event can never
        // advertise a public registration URL — the projection derives
        // that from the event, and this is where members-only events
        // stop being registerable.
        self.guest_registration_enabled = false;
    }

    /// Whether `member` may see this event on the member surfaces (the
    /// events list, the dashboard's upcoming widget, RSVP).
    ///
    /// `AdminOnly` is the level an org uses for events the membership is
    /// not meant to know about, so a non-admin gets neither the row nor
    /// any of its content; an admin keeps seeing everything, since the
    /// same content is already theirs to read in the admin surface.
    /// One home for the rule so the decision isn't re-derived (and
    /// re-broken) per call site — mirrors [`Event::publicly_registerable`].
    pub fn visible_to_member(&self, member: &crate::domain::Member) -> bool {
        member.is_admin || self.visibility != EventVisibility::AdminOnly
    }

    /// The absolute URL of the Coterie-hosted public registration page,
    /// or `None` when the event is not publicly registerable.
    ///
    /// Resolving it server-side is the point: whether an event may be
    /// publicly registered is an authorization rule, so the answer is
    /// emitted rather than the ingredients. A consumer decides by testing
    /// this for presence — never by re-deriving the rule from prices and
    /// visibility flags, which would be a second implementation of it.
    pub fn registration_url(&self, base_url: &str) -> Option<String> {
        self.publicly_registerable().then(|| {
            format!(
                "{}/events/{}/register",
                base_url.trim_end_matches('/'),
                self.id,
            )
        })
    }

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

    /// Whether this event is still upcoming at `now` — convenience
    /// wrapper over [`is_upcoming`] for callers holding an event whose
    /// fields still carry the STORED wall-clock.
    ///
    /// A surface that has already replaced those fields with derived
    /// instants (the public feed's `derive_utc_instants`) must call the
    /// free function directly: this one derives, and deriving an
    /// already-derived instant shifts it a second time by the org's
    /// offset.
    pub fn is_upcoming(&self, now: DateTime<Utc>) -> bool {
        is_upcoming(self.start_utc(), self.end_utc(), now)
    }
}

/// How long an event with no recorded `end_time` stays upcoming past its
/// start.
///
/// A missing end time means the end is UNKNOWN, not that the event ends
/// the instant it begins — treating the absent value as a zero-length
/// event would drop the listing at exactly the moment the event is
/// happening, for precisely the events least likely to have an end
/// recorded.
///
/// Deliberately a constant and not a setting: the field that answers
/// "how long does this run" is `end_time`, and the fix for a bad guess is
/// to record one. Two hours is what this org's own recurring evenings
/// actually run (19:00–21:00).
pub const UNKNOWN_END_GRACE_HOURS: i64 = 2;

/// The grace period as a [`Duration`]. See [`UNKNOWN_END_GRACE_HOURS`].
pub fn unknown_end_grace() -> Duration {
    Duration::hours(UNKNOWN_END_GRACE_HOURS)
}

/// Whether an event is still upcoming at `now`: it is, until it ends.
///
/// The single home for the rule — every surface that lists or counts
/// upcoming events reads this instead of writing `start > now`, which is
/// how three copies of the comparison drifted apart before.
///
/// Takes already-derived UTC **instants** rather than an `&Event` on
/// purpose: `Event::start_time`/`end_time` normally hold a wall-clock,
/// but the public feed overwrites them in place with derived instants
/// before filtering, so a predicate that re-derived would apply the zone
/// offset twice on exactly the surface where an in-progress event is most
/// visible. `now` is a parameter rather than a clock read for the same
/// reason: a clock-reading predicate can't be tested at its boundary.
pub fn is_upcoming(
    start_utc: DateTime<Utc>,
    end_utc: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> bool {
    end_utc.unwrap_or(start_utc + unknown_end_grace()) > now
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
    /// What a member pays for a pass to the whole series, in cents. `0`
    /// means free — a *known* price, not an unknown one, same as
    /// [`Event::member_price_cents`]. Above `0` makes this a paid class,
    /// which requires an `until_date` (see [`EventSeries::validate_pricing`]).
    pub member_price_cents: i64,
    /// What a non-member pays for a pass. `0` means free, on the same
    /// terms. Says nothing about *whether* non-members may enroll —
    /// that's [`EventSeries::guest_registration_enabled`].
    pub guest_price_cents: i64,
    /// Whether non-members may enroll through the public class page.
    pub guest_registration_enabled: bool,
    /// How many people may hold a pass to this class. `None` = uncapped.
    /// This is a series-level number: twelve seats in the course, not
    /// twelve seats each night.
    pub max_enrollments: Option<i32>,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The pricing/capacity subset of a series, as the create + update paths
/// carry it. Defaults to free-and-uncapped, which is what every series
/// was before passes existed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SeriesPassPricing {
    pub member_price_cents: i64,
    pub guest_price_cents: i64,
    pub guest_registration_enabled: bool,
    pub max_enrollments: Option<i32>,
}

impl SeriesPassPricing {
    /// True when a pass costs a member money — the paid-class test,
    /// available before the series row exists.
    pub fn is_paid_class(&self) -> bool {
        self.member_price_cents > 0
    }
}

impl EventSeries {
    /// True when a pass to this series costs a member money. The single
    /// home for the free-vs-paid-class rule, so templates and services
    /// don't re-derive `> 0` and drift on the boundary.
    pub fn is_paid_class(&self) -> bool {
        self.member_price_cents > 0
    }

    /// True when a pass costs a non-member money. Mirrors
    /// [`EventSeries::is_paid_class`]; `false` means public enrollment is
    /// free, NOT that it is closed.
    pub fn is_paid_for_guests(&self) -> bool {
        self.guest_price_cents > 0
    }

    /// Whether the public class page and endpoint serve this series: the
    /// org opened the door AND its occurrences are `Public`. A series row
    /// carries no visibility of its own — that lives per occurrence — so
    /// the caller supplies one to read it from. Mirrors
    /// [`Event::publicly_registerable`], and for the same reason: one home
    /// for the rule so the 404 decision isn't re-derived per call site.
    pub fn publicly_enrollable(&self, occurrence: &Event) -> bool {
        self.guest_registration_enabled && occurrence.visibility == EventVisibility::Public
    }

    /// The pricing/capacity subset, for handing to an update path.
    pub fn pricing(&self) -> SeriesPassPricing {
        SeriesPassPricing {
            member_price_cents: self.member_price_cents,
            guest_price_cents: self.guest_price_cents,
            guest_registration_enabled: self.guest_registration_enabled,
            max_enrollments: self.max_enrollments,
        }
    }
}

/// Validate pass pricing against the series' end date.
///
/// Two rules, both enforced wherever a series is created or repriced:
///
///   - **A priced class must be bounded.** A flat price buying unlimited
///     future sessions is a subscription, not a pass, and Coterie already
///     has recurring billing for subscriptions.
///   - **Prices are bounded like every other amount** — no negatives, and
///     nothing above the single-payment cap. Zero is accepted and stored
///     as zero.
pub fn validate_pass_pricing(
    pricing: &SeriesPassPricing,
    until_date: Option<DateTime<Utc>>,
) -> Result<(), String> {
    for (label, cents) in [
        ("Member pass price", pricing.member_price_cents),
        ("Guest pass price", pricing.guest_price_cents),
    ] {
        if cents < 0 {
            return Err(format!("{} can't be negative", label));
        }
        if cents > crate::domain::MAX_PAYMENT_CENTS {
            return Err(format!(
                "{} exceeds the ${} cap on a single payment",
                label,
                crate::domain::MAX_PAYMENT_CENTS / 100,
            ));
        }
    }

    if (pricing.member_price_cents > 0 || pricing.guest_price_cents > 0) && until_date.is_none() {
        return Err(
            "A priced class must have an end date — set the series' \"repeat until\" date \
             before giving it a pass price. A flat price on an open-ended series would sell \
             unlimited future sessions for one payment, which is a subscription rather than \
             a pass."
                .to_string(),
        );
    }

    Ok(())
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

/// Who holds a seat. A sum type for the same reason [`crate::domain::Payer`]
/// is one: a row is a member's seat or a guest's seat, never both and never
/// neither, and three loose optionals (`member_id`, `guest_name`,
/// `guest_email`) would let a caller construct the states the DB CHECK
/// rejects. The DB stores the three nullable columns; the repository maps
/// them to (and from) this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Attendee {
    Member(Uuid),
    /// A non-member registering through the public page. The identity is
    /// unverified input, captured for the roster and the confirmation
    /// email — it is deliberately NOT matched against the member
    /// directory (see the paid-events spec).
    Guest {
        name: String,
        email: String,
    },
}

impl Attendee {
    /// The member uuid, or `None` for a guest. Mirrors
    /// [`crate::domain::Payer::member_id`].
    pub fn member_id(&self) -> Option<Uuid> {
        match self {
            Attendee::Member(id) => Some(*id),
            Attendee::Guest { .. } => None,
        }
    }

    /// The guest email, or `None` for a member. The seat's identity
    /// column on the guest side.
    pub fn guest_email(&self) -> Option<&str> {
        match self {
            Attendee::Member(_) => None,
            Attendee::Guest { email, .. } => Some(email),
        }
    }

    pub fn guest_name(&self) -> Option<&str> {
        match self {
            Attendee::Member(_) => None,
            Attendee::Guest { name, .. } => Some(name),
        }
    }

    /// The payer this attendee pays as. A guest is a non-member payer —
    /// the `PublicDonor` variant, whose structure ("non-member payer
    /// whose identity we captured for receipts") already describes a
    /// guest registrant exactly.
    pub fn as_payer(&self) -> crate::domain::Payer {
        match self {
            Attendee::Member(id) => crate::domain::Payer::Member(*id),
            Attendee::Guest { name, email } => crate::domain::Payer::PublicDonor {
                name: name.clone(),
                email: email.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventAttendance {
    /// Surrogate key (migration 043). A guest row can't be identified by
    /// `(event_id, member_id)` because it has no member.
    pub id: Uuid,
    pub event_id: Uuid,
    /// Who holds the seat — a member or a guest. See [`Attendee`].
    pub attendee: Attendee,
    pub status: AttendanceStatus,
    pub registered_at: DateTime<Utc>,
    pub attended: bool,
    /// The event-fee payment holding this seat. `None` for free RSVPs.
    pub payment_id: Option<Uuid>,
}

/// Somebody's pass to a whole series. Deliberately the same shape as
/// [`EventAttendance`] — an [`Attendee`] identity, the payment holding
/// it, and the same [`AttendanceStatus`] lifecycle — because it IS the
/// same lifecycle at a different scope. A second, parallel status enum
/// would drift from this one on exactly the double-pay / race / refund
/// edges the paid-events capability exists to get right.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeriesEnrollment {
    pub id: Uuid,
    pub series_id: Uuid,
    /// Who holds the pass — a member or a guest. See [`Attendee`].
    pub enrollee: Attendee,
    pub status: AttendanceStatus,
    pub enrolled_at: DateTime<Utc>,
    /// The series-pass payment holding this enrollment. `None` for a free
    /// class.
    pub payment_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT")]
pub enum AttendanceStatus {
    Registered,
    Waitlisted,
    Cancelled,
    /// Seat held while the member is at Stripe Checkout. Holds capacity
    /// only while its linked payment is still `Pending` — an abandoned
    /// checkout releases the seat by virtue of the payment being flipped
    /// to `Failed`, without the row having to be deleted.
    PendingPayment,
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
            member_price_cents: 0,
            guest_price_cents: 0,
            guest_registration_enabled: false,
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

    // Past/upcoming status must compare the DERIVED instant to `now`, not
    // the raw wall-clock. A 7 PM EST event (19:00 wall-clock, 00:00Z next
    // day) at a `now` after 19:00Z but before the true start is still
    // upcoming; comparing the raw wall-clock would wrongly call it past.
    #[test]
    fn past_status_uses_derived_instant_not_wallclock() {
        let e = event_at(wall(2026, 1, 23, 19, 0), "America/New_York");
        // 19:30Z on the event day: after the raw wall-clock (19:00Z) but
        // ~4.5h before the true start (00:00Z the 24th, EST is UTC-5).
        let now = Utc.with_ymd_and_hms(2026, 1, 23, 19, 30, 0).unwrap();
        assert_eq!(e.start_utc().to_rfc3339(), "2026-01-24T00:00:00+00:00");
        // Correct: derived instant is still in the future → upcoming.
        assert!(e.start_utc() > now, "derived instant should be upcoming");
        // The bug: raw wall-clock would flip it to past ~4.5h early.
        assert!(
            e.start_time <= now,
            "raw wall-clock (19:00Z) mis-reads as past against 19:30Z now"
        );
    }

    fn member(is_admin: bool) -> crate::domain::Member {
        crate::domain::Member {
            id: Uuid::new_v4(),
            email: "m@example.com".into(),
            username: "m".into(),
            full_name: "M".into(),
            status: crate::domain::MemberStatus::Active,
            membership_type_id: Uuid::new_v4(),
            joined_at: Utc::now(),
            expires_at: None,
            dues_paid_until: None,
            bypass_dues: false,
            is_admin,
            notes: None,
            stripe_customer_id: None,
            stripe_subscription_id: None,
            billing_mode: Default::default(),
            email_verified_at: None,
            dues_reminder_sent_at: None,
            discord_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    // AdminOnly is the level for events the membership is not meant to
    // know about: a non-admin must not see the row at all, while an
    // admin keeps seeing it. Public/MembersOnly stay visible to everyone.
    #[test]
    fn admin_only_event_is_hidden_from_non_admin() {
        let mut e = event_at(wall(2026, 7, 23, 19, 0), "UTC");
        e.visibility = EventVisibility::AdminOnly;
        assert!(!e.visible_to_member(&member(false)));
        assert!(e.visible_to_member(&member(true)));

        for v in [EventVisibility::Public, EventVisibility::MembersOnly] {
            e.visibility = v.clone();
            assert!(
                e.visible_to_member(&member(false)),
                "{v:?} must stay visible"
            );
        }
    }

    // 3.1 — the predicate flips at the END instant, not the start. An
    // event in progress is still upcoming; one a second past its end is
    // not. Bounds are derived from the event under test rather than
    // hardcoded, so this can't become a day-of-week flake.
    #[test]
    fn is_upcoming_flips_at_the_end_instant() {
        for zone in ["UTC", "America/New_York", "Australia/Sydney"] {
            let e = event_at(wall(2026, 7, 23, 19, 0), zone);
            let end = e.end_utc().expect("fixture records an end");
            assert!(
                e.is_upcoming(end - Duration::seconds(1)),
                "{zone}: one second before the end is still upcoming"
            );
            assert!(
                !e.is_upcoming(end + Duration::seconds(1)),
                "{zone}: one second after the end is not"
            );
            // The whole point: started, not ended.
            assert!(
                e.is_upcoming(e.start_utc() + Duration::minutes(30)),
                "{zone}: an event in progress stays upcoming"
            );
        }
    }

    // 3.1 — a missing end time means the end is UNKNOWN, so the grace
    // period decides, and it is two hours.
    #[test]
    fn is_upcoming_uses_the_grace_period_when_no_end_is_recorded() {
        assert_eq!(UNKNOWN_END_GRACE_HOURS, 2, "the grace period is two hours");
        for zone in ["UTC", "America/New_York"] {
            let mut e = event_at(wall(2026, 7, 23, 19, 0), zone);
            e.end_time = None;
            let start = e.start_utc();
            assert!(
                e.is_upcoming(start + Duration::minutes(30)),
                "{zone}: 30 minutes in, well inside the grace period"
            );
            assert!(
                e.is_upcoming(start + unknown_end_grace() - Duration::seconds(1)),
                "{zone}: one second before the grace expires"
            );
            assert!(
                !e.is_upcoming(start + unknown_end_grace() + Duration::seconds(1)),
                "{zone}: one second after"
            );
        }
    }

    // 3.1 — a naive wall-clock comparison answers by the org's offset.
    // At a `now` past the RAW stored end but hours before the derived
    // one, the event must still read as upcoming.
    #[test]
    fn is_upcoming_resolves_the_zone_not_the_raw_wallclock() {
        // 19:00–20:00 on Jan 23 in New York (EST, UTC-5) → 00:00Z–01:00Z
        // on the 24th.
        let e = event_at(wall(2026, 1, 23, 19, 0), "America/New_York");
        let raw_end = e.end_time.expect("fixture records an end");
        let now = raw_end + Duration::hours(1);
        assert!(now > raw_end, "now is past the raw wall-clock end (20:00Z)");
        assert!(
            e.is_upcoming(now),
            "but four hours before the derived end (01:00Z) — still upcoming"
        );
        assert!(!e.is_upcoming(e.end_utc().unwrap() + Duration::seconds(1)));
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
