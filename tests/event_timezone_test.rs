//! Integration tests for the event-timezone-correctness change.
//!
//! Covers the pieces that need a real DB + services (the pure
//! (wall-clock, zone) → UTC derivation math lives in unit tests under
//! `src/domain/event.rs`):
//!
//!   - 7.4 the setting validator rejects an unknown IANA zone and
//!     retains the previous value
//!   - 7.1 an event entered at 7 PM stores local `19:00` + zone, the
//!     admin surface re-renders `19:00`, and the read path derives
//!     `23:00:00Z` in July for America/New_York
//!   - 5.2 a weekly evening series spanning a DST boundary keeps a
//!     constant local time; its derived UTC instants differ by an hour
//!
//! Run: cargo test --test event_timezone_test

use std::sync::Arc;

use chrono::{DateTime, NaiveDate, NaiveDateTime, Timelike, Utc};
use coterie::{
    auth::SecretCrypto,
    domain::{
        CreateMemberRequest, EventType, EventVisibility, Recurrence, UpdateSettingRequest,
        WeekdayCode,
    },
    integrations::IntegrationManager,
    repository::{
        EventRepository, EventSeriesRepository, MemberRepository, SqliteEventRepository,
        SqliteEventSeriesRepository, SqliteMemberRepository,
    },
    service::{
        audit_service::AuditService,
        event_admin_service::{CreateEventInput, EventAdminService},
        recurring_event_service::RecurringEventService,
        settings_service::SettingsService,
    },
};
use sqlx::SqlitePool;
use uuid::Uuid;

mod common;
use common::fresh_pool;

fn wall(y: i32, mo: u32, d: u32, h: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(y, mo, d)
        .unwrap()
        .and_hms_opt(h, 0, 0)
        .unwrap()
}

fn as_container(naive: NaiveDateTime) -> DateTime<Utc> {
    DateTime::from_naive_utc_and_offset(naive, Utc)
}

async fn make_actor(pool: &SqlitePool) -> Uuid {
    SqliteMemberRepository::new(pool.clone())
        .create(CreateMemberRequest {
            email: format!("a-{}@example.com", Uuid::new_v4()),
            username: format!("u_{}", Uuid::new_v4().simple()),
            full_name: "Admin".to_string(),
            password: "p4ssword_long_enough".to_string(),
            membership_type_id: None,
            ..Default::default()
        })
        .await
        .unwrap()
        .id
}

fn make_event_admin(pool: &SqlitePool) -> EventAdminService {
    let event_repo: Arc<dyn EventRepository> = Arc::new(SqliteEventRepository::new(pool.clone()));
    let series_repo: Arc<dyn EventSeriesRepository> =
        Arc::new(SqliteEventSeriesRepository::new(pool.clone()));
    let recurring = Arc::new(RecurringEventService::new(
        event_repo.clone(),
        series_repo.clone(),
        pool.clone(),
    ));
    let audit = Arc::new(AuditService::new(pool.clone()));
    let integrations = Arc::new(IntegrationManager::new());
    EventAdminService::new(event_repo, series_repo, recurring, audit, integrations)
}

fn settings(pool: &SqlitePool) -> SettingsService {
    let crypto = Arc::new(SecretCrypto::new("test-secret-please-ignore"));
    SettingsService::new(pool.clone(), crypto)
}

fn single_input(start: NaiveDateTime, zone: &str) -> CreateEventInput {
    CreateEventInput {
        title: "Launch Party".to_string(),
        description: "7 PM sharp".to_string(),
        event_type: EventType::Social,
        event_type_id: None,
        visibility: EventVisibility::Public,
        start_time: as_container(start),
        end_time: None,
        timezone: zone.to_string(),
        location: None,
        max_attendees: None,
        rsvp_required: false,
        image_url: None,
        recurrence: None,
        recurrence_until: None,
    }
}

// 7.4 — an unknown IANA name is rejected and the previous value stays.
#[tokio::test]
async fn unknown_timezone_is_rejected_and_previous_retained() {
    let pool = fresh_pool().await;
    let actor = make_actor(&pool).await;
    let svc = settings(&pool);

    // Seeded default is UTC.
    assert_eq!(svc.get_value("org.timezone").await.unwrap(), "UTC");

    let err = svc
        .update_setting(
            "org.timezone",
            UpdateSettingRequest {
                value: "Mars/Olympus_Mons".to_string(),
                reason: None,
            },
            actor,
        )
        .await;
    assert!(err.is_err(), "unknown zone must be rejected");
    // Previous value retained — no partial write.
    assert_eq!(svc.get_value("org.timezone").await.unwrap(), "UTC");

    // A real zone is accepted.
    svc.update_setting(
        "org.timezone",
        UpdateSettingRequest {
            value: "America/New_York".to_string(),
            reason: None,
        },
        actor,
    )
    .await
    .expect("valid zone accepted");
    assert_eq!(
        svc.get_value("org.timezone").await.unwrap(),
        "America/New_York"
    );
    assert_eq!(svc.org_timezone().await.name(), "America/New_York");
}

// 7.1 — round-trip: stored wall-clock + zone, admin re-renders 19:00,
// read path derives 23:00:00Z in July for America/New_York.
#[tokio::test]
async fn roundtrip_stores_wallclock_and_derives_utc() {
    let pool = fresh_pool().await;
    let actor = make_actor(&pool).await;
    let admin = make_event_admin(&pool);
    let settings = settings(&pool);

    settings
        .update_setting(
            "org.timezone",
            UpdateSettingRequest {
                value: "America/New_York".to_string(),
                reason: None,
            },
            actor,
        )
        .await
        .unwrap();
    let zone = settings.org_timezone().await.name().to_string();

    let created = admin
        .create(actor, single_input(wall(2026, 7, 23, 19), &zone))
        .await
        .expect("create event");

    // Re-read from the DB to prove persistence, not in-memory state.
    let repo = SqliteEventRepository::new(pool.clone());
    let stored = repo.find_by_id(created.id).await.unwrap().unwrap();

    // Stored authoritative fields: naive wall-clock + zone (no shift).
    assert_eq!(stored.start_time.naive_utc(), wall(2026, 7, 23, 19));
    assert_eq!(stored.timezone, "America/New_York");

    // Admin surface re-renders the wall-clock exactly.
    assert_eq!(
        stored.start_time.format("%Y-%m-%dT%H:%M").to_string(),
        "2026-07-23T19:00"
    );

    // Read path (public/iCal) derives the true instant.
    assert_eq!(stored.start_utc().to_rfc3339(), "2026-07-23T23:00:00+00:00");
}

// 5.2 — a weekly 7 PM series spanning the Nov 2026 DST fall-back keeps a
// constant local time; the derived UTC instants differ by an hour.
#[tokio::test]
async fn weekly_series_survives_dst_boundary() {
    let pool = fresh_pool().await;
    let actor = make_actor(&pool).await;
    let admin = make_event_admin(&pool);
    let event_repo = SqliteEventRepository::new(pool.clone());

    // 2026-10-20 is a Tuesday at 19:00 local. DST ends 2026-11-01, so
    // occurrence #2 (Oct 27) is EDT and #3 (Nov 3) is EST.
    let anchor = wall(2026, 10, 20, 19);
    let mut input = single_input(anchor, "America/New_York");
    input.recurrence = Some(Recurrence::WeeklyByDay {
        interval: 1,
        weekdays: vec![WeekdayCode::Tue],
    });

    let created = admin.create(actor, input).await.expect("create series");
    let series_id = created.series_id.expect("series occurrence");

    let occ2 = event_repo
        .find_by_series_and_index(series_id, 2)
        .await
        .unwrap()
        .expect("occurrence 2 (Oct 27, EDT)");
    let occ3 = event_repo
        .find_by_series_and_index(series_id, 3)
        .await
        .unwrap()
        .expect("occurrence 3 (Nov 3, EST)");

    // Wall-clock is a constant 19:00 across the boundary.
    assert_eq!(occ2.start_time.hour(), 19);
    assert_eq!(occ3.start_time.hour(), 19);
    assert_eq!(occ2.timezone, "America/New_York");

    // Derived instants: EDT (UTC-4) → 23:00Z, EST (UTC-5) → 00:00Z next
    // day — an hour apart in wall-clock-of-UTC terms.
    assert_eq!(occ2.start_utc().to_rfc3339(), "2026-10-27T23:00:00+00:00");
    assert_eq!(occ3.start_utc().to_rfc3339(), "2026-11-04T00:00:00+00:00");

    // The per-week gap is 169h (168 + the extra fall-back hour), proving
    // the instants shifted by an hour rather than staying a fixed 168h.
    let delta = occ3.start_utc() - occ2.start_utc();
    assert_eq!(delta.num_hours(), 169);
}

// 7.3 — the annotation shifts no time value. A "legacy" row (inserted
// without the new zone column, as pre-migration data was) keeps its
// stored wall-clock and is annotated with the default zone; its rendered
// local time is identical before and after.
#[tokio::test]
async fn annotation_preserves_wallclock_and_defaults_zone() {
    let pool = fresh_pool().await;
    let actor = make_actor(&pool).await;

    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO events (id, title, description, event_type, visibility, start_time, created_by) \
         VALUES (?, 'Legacy', '', 'Meeting', 'Public', '2026-07-23 19:00:00', ?)",
    )
    .bind(id.to_string())
    .bind(actor.to_string())
    .execute(&pool)
    .await
    .unwrap();

    let stored = SqliteEventRepository::new(pool.clone())
        .find_by_id(id)
        .await
        .unwrap()
        .unwrap();

    // Stored wall-clock unchanged; annotated with the column default zone.
    assert_eq!(stored.start_time.naive_utc(), wall(2026, 7, 23, 19));
    assert_eq!(stored.timezone, "UTC");
    // Rendered local time identical to what was stored.
    assert_eq!(
        stored.start_time.format("%Y-%m-%dT%H:%M").to_string(),
        "2026-07-23T19:00"
    );
}
