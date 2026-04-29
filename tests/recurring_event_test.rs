//! Integration tests for `RecurringEventService`. Hits a real
//! in-memory SQLite + migrations; constructs the service against
//! `SqliteEventRepository` + `SqliteEventSeriesRepository` and
//! exercises:
//!
//!   - initial materialization (creates ~12 months of occurrences)
//!   - horizon extension (adds the missing tail when called again)
//!   - until_date capping (no occurrences past the cutoff)
//!   - edit-this-and-future (updates only rows from a chosen point)
//!   - cancel-this-one (hard delete of a single row)
//!   - end-the-series (delete future + set until_date)
//!
//! Run: cargo test --test recurring_event_test

use std::sync::Arc;

use chrono::{DateTime, Datelike, Duration, TimeZone, Utc, Weekday};
use coterie::{
    domain::{
        CreateMemberRequest, Event, EventType, EventVisibility, MembershipType,
        Recurrence, WeekdayCode,
    },
    repository::{
        EventRepository, EventSeriesRepository, MemberRepository,
        SqliteEventRepository, SqliteEventSeriesRepository, SqliteMemberRepository,
    },
    service::recurring_event_service::RecurringEventService,
};
use sqlx::{Executor, SqlitePool};
use uuid::Uuid;

async fn fresh_pool() -> SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .after_connect(|conn, _| {
            Box::pin(async move {
                conn.execute("PRAGMA foreign_keys = ON").await?;
                Ok(())
            })
        })
        .connect("sqlite::memory:")
        .await
        .expect("connect to :memory:");
    sqlx::migrate!("./migrations").run(&pool).await.expect("migrate");
    pool
}

struct H {
    pool: SqlitePool,
    event_repo: Arc<dyn EventRepository>,
    series_repo: Arc<dyn EventSeriesRepository>,
    service: RecurringEventService,
    creator: Uuid,
}

async fn build() -> H {
    let pool = fresh_pool().await;
    let event_repo: Arc<dyn EventRepository> =
        Arc::new(SqliteEventRepository::new(pool.clone()));
    let series_repo: Arc<dyn EventSeriesRepository> =
        Arc::new(SqliteEventSeriesRepository::new(pool.clone()));
    let service = RecurringEventService::new(
        event_repo.clone(),
        series_repo.clone(),
        pool.clone(),
    );

    // Need a member for `created_by`.
    let mr = SqliteMemberRepository::new(pool.clone());
    let m = mr
        .create(CreateMemberRequest {
            email: format!("a-{}@example.com", Uuid::new_v4()),
            username: format!("u_{}", Uuid::new_v4().simple()),
            full_name: "Test Admin".to_string(),
            password: "p4ssword_long_enough".to_string(),
            membership_type: MembershipType::Regular,
        })
        .await
        .unwrap();

    H { pool, event_repo, series_repo, service, creator: m.id }
}

fn template(creator: Uuid, start: DateTime<Utc>) -> Event {
    Event {
        id: Uuid::new_v4(),
        title: "Tuesday Coffee".to_string(),
        description: "Weekly hangout".to_string(),
        event_type: EventType::Social,
        event_type_id: None,
        visibility: EventVisibility::MembersOnly,
        start_time: start,
        end_time: Some(start + Duration::hours(2)),
        location: Some("HQ".to_string()),
        max_attendees: Some(20),
        rsvp_required: true,
        image_url: None,
        created_by: creator,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        series_id: None,
        occurrence_index: None,
    }
}

async fn count_in_series(pool: &SqlitePool, series_id: Uuid) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM events WHERE series_id = ?",
    )
    .bind(series_id.to_string())
    .fetch_one(pool)
    .await
    .unwrap()
}

// --------------------------------------------------------------------
// Initial materialization
// --------------------------------------------------------------------

#[tokio::test]
async fn weekly_creates_about_52_occurrences() {
    let h = build().await;
    let anchor = Utc.with_ymd_and_hms(2026, 5, 5, 18, 0, 0).unwrap(); // Tue
    let rule = Recurrence::WeeklyByDay {
        interval: 1,
        weekdays: vec![WeekdayCode::Tue],
    };

    let created = h.service.create_series_with_initial_materialization(
        rule, template(h.creator, anchor), None, h.creator,
    ).await.expect("create");

    // 52 weekly occurrences within 52 weeks ahead. Generation excludes
    // anything ≥ target, so the count is typically exactly 52.
    assert!(
        (50..=53).contains(&created.occurrences.len()),
        "got {} occurrences", created.occurrences.len(),
    );
    let series = h.series_repo.find_by_id(created.series.id).await.unwrap().unwrap();
    assert!(series.materialized_through > anchor);
    // First occurrence is the anchor.
    assert_eq!(created.occurrences[0].start_time, anchor);
    // Each occurrence inherits template fields.
    for occ in &created.occurrences {
        assert_eq!(occ.title, "Tuesday Coffee");
        assert_eq!(occ.location.as_deref(), Some("HQ"));
        assert_eq!(occ.start_time.weekday(), Weekday::Tue);
        assert_eq!(occ.series_id, Some(created.series.id));
        assert_eq!(occ.end_time.unwrap() - occ.start_time, Duration::hours(2));
    }
    // Indices are 1..=N and contiguous.
    let mut idxs: Vec<i32> = created.occurrences.iter()
        .map(|o| o.occurrence_index.unwrap()).collect();
    idxs.sort();
    assert_eq!(idxs, (1..=created.occurrences.len() as i32).collect::<Vec<_>>());
}

#[tokio::test]
async fn until_date_caps_materialization() {
    let h = build().await;
    let anchor = Utc.with_ymd_and_hms(2026, 5, 5, 18, 0, 0).unwrap(); // Tue
    let until = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap(); // ~3 months
    let rule = Recurrence::WeeklyByDay {
        interval: 1,
        weekdays: vec![WeekdayCode::Tue],
    };

    let created = h.service.create_series_with_initial_materialization(
        rule, template(h.creator, anchor), Some(until), h.creator,
    ).await.unwrap();

    // ~13 Tuesdays in May 5 → Aug 1.
    assert!(
        (10..=13).contains(&created.occurrences.len()),
        "got {} occurrences with 3-month cap", created.occurrences.len(),
    );
    for occ in &created.occurrences {
        assert!(occ.start_time < until,
            "occurrence at {} crosses until_date {}", occ.start_time, until);
    }
}

// --------------------------------------------------------------------
// Horizon extension
// --------------------------------------------------------------------

#[tokio::test]
async fn extend_horizon_adds_missing_tail() {
    let h = build().await;
    let anchor = Utc.with_ymd_and_hms(2026, 5, 5, 18, 0, 0).unwrap();
    let initial_until = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap();
    let rule = Recurrence::WeeklyByDay {
        interval: 1, weekdays: vec![WeekdayCode::Tue],
    };

    // Create with a 3-month cap so we can extend later.
    let created = h.service.create_series_with_initial_materialization(
        rule, template(h.creator, anchor), Some(initial_until), h.creator,
    ).await.unwrap();
    let initial_count = created.occurrences.len();

    // Lift the cap by clearing until_date directly so extend_horizon
    // can roll forward. (The "end the series" admin action sets
    // until_date; the inverse — "open up an existing series" — isn't
    // a supported flow, but we test the underlying mechanic.)
    sqlx::query("UPDATE event_series SET until_date = NULL WHERE id = ?")
        .bind(created.series.id.to_string())
        .execute(&h.pool).await.unwrap();
    let series = h.series_repo.find_by_id(created.series.id).await.unwrap().unwrap();

    // Roll forward to a year past anchor — adds the missing 9 months.
    let target = anchor + Duration::weeks(52);
    let added = h.service.extend_horizon(&series, target).await.unwrap();
    assert!(added > 30, "expected significant tail-fill, got {}", added);

    let total = count_in_series(&h.pool, created.series.id).await as usize;
    assert!(
        (initial_count + added as usize) == total,
        "initial {} + added {} != total {}", initial_count, added, total,
    );

    // Extending again with the same target is a no-op.
    let series = h.series_repo.find_by_id(created.series.id).await.unwrap().unwrap();
    let added_again = h.service.extend_horizon(&series, target).await.unwrap();
    assert_eq!(added_again, 0, "horizon-extend must be idempotent");
}

#[tokio::test]
async fn extend_horizon_respects_until_date() {
    let h = build().await;
    let anchor = Utc.with_ymd_and_hms(2026, 5, 5, 18, 0, 0).unwrap();
    let until = Utc.with_ymd_and_hms(2026, 6, 30, 0, 0, 0).unwrap();
    let rule = Recurrence::WeeklyByDay {
        interval: 1, weekdays: vec![WeekdayCode::Tue],
    };
    let created = h.service.create_series_with_initial_materialization(
        rule, template(h.creator, anchor), Some(until), h.creator,
    ).await.unwrap();

    // Try to extend past until_date — must be a no-op.
    let series = h.series_repo.find_by_id(created.series.id).await.unwrap().unwrap();
    let added = h.service.extend_horizon(
        &series,
        anchor + Duration::weeks(52),
    ).await.unwrap();
    assert_eq!(added, 0);

    // No occurrence past until_date.
    let max_start: chrono::NaiveDateTime = sqlx::query_scalar(
        "SELECT MAX(start_time) FROM events WHERE series_id = ?",
    )
    .bind(created.series.id.to_string())
    .fetch_one(&h.pool).await.unwrap();
    let max_start: DateTime<Utc> = DateTime::from_naive_utc_and_offset(max_start, Utc);
    assert!(max_start < until);
}

// --------------------------------------------------------------------
// Edit-this-and-future
// --------------------------------------------------------------------

#[tokio::test]
async fn update_series_occurrences_from_only_touches_future_rows() {
    let h = build().await;
    let anchor = Utc.with_ymd_and_hms(2026, 5, 5, 18, 0, 0).unwrap();
    let rule = Recurrence::WeeklyByDay {
        interval: 1, weekdays: vec![WeekdayCode::Tue],
    };
    let created = h.service.create_series_with_initial_materialization(
        rule, template(h.creator, anchor),
        Some(Utc.with_ymd_and_hms(2026, 7, 1, 0, 0, 0).unwrap()),
        h.creator,
    ).await.unwrap();

    // Pivot: the 3rd occurrence. Edit IT and onwards.
    let pivot_idx = 2; // 0-based — third occurrence
    let pivot = &created.occurrences[pivot_idx];
    let pivot_start = pivot.start_time;

    // Build a template with new title/description.
    let mut updated = template(h.creator, pivot_start);
    updated.title = "Espresso Hour".to_string();
    updated.description = "Renamed".to_string();

    let n = h.event_repo.update_series_occurrences_from(
        created.series.id, pivot_start, &updated,
    ).await.unwrap();
    assert!(n >= 1);

    // Pre-pivot rows must be untouched.
    for occ in &created.occurrences[..pivot_idx] {
        let cur = h.event_repo.find_by_id(occ.id).await.unwrap().unwrap();
        assert_eq!(cur.title, "Tuesday Coffee", "pre-pivot {} got renamed", occ.id);
    }
    // Pivot + post-pivot rows must reflect the new title.
    for occ in &created.occurrences[pivot_idx..] {
        let cur = h.event_repo.find_by_id(occ.id).await.unwrap().unwrap();
        assert_eq!(cur.title, "Espresso Hour", "post-pivot {} not renamed", occ.id);
    }
}

// --------------------------------------------------------------------
// Cancel one occurrence (hard delete; series + siblings unaffected)
// --------------------------------------------------------------------

#[tokio::test]
async fn delete_one_occurrence_leaves_siblings_intact() {
    let h = build().await;
    let anchor = Utc.with_ymd_and_hms(2026, 5, 5, 18, 0, 0).unwrap();
    let rule = Recurrence::WeeklyByDay {
        interval: 1, weekdays: vec![WeekdayCode::Tue],
    };
    let created = h.service.create_series_with_initial_materialization(
        rule, template(h.creator, anchor),
        Some(Utc.with_ymd_and_hms(2026, 7, 1, 0, 0, 0).unwrap()),
        h.creator,
    ).await.unwrap();
    let initial = count_in_series(&h.pool, created.series.id).await;

    // Cancel one mid-series occurrence.
    let target = &created.occurrences[3];
    h.event_repo.delete(target.id).await.unwrap();

    let after = count_in_series(&h.pool, created.series.id).await;
    assert_eq!(after, initial - 1);
    // Series row is intact.
    assert!(h.series_repo.find_by_id(created.series.id).await.unwrap().is_some());
    // Other occurrences still resolvable.
    let neighbor = h.event_repo.find_by_id(created.occurrences[2].id).await.unwrap();
    assert!(neighbor.is_some());
}

// --------------------------------------------------------------------
// End the series after a date
// --------------------------------------------------------------------

#[tokio::test]
async fn end_series_after_date_deletes_future_only() {
    let h = build().await;
    let anchor = Utc.with_ymd_and_hms(2026, 5, 5, 18, 0, 0).unwrap();
    let rule = Recurrence::WeeklyByDay {
        interval: 1, weekdays: vec![WeekdayCode::Tue],
    };
    let created = h.service.create_series_with_initial_materialization(
        rule, template(h.creator, anchor),
        Some(Utc.with_ymd_and_hms(2027, 5, 5, 0, 0, 0).unwrap()),
        h.creator,
    ).await.unwrap();
    let total_before = count_in_series(&h.pool, created.series.id).await;

    // End the series after the 5th occurrence — only 5 stay.
    let cutoff = created.occurrences[4].start_time;
    let deleted = h.event_repo
        .delete_series_occurrences_after(created.series.id, cutoff)
        .await.unwrap();
    h.series_repo.set_until_date(created.series.id, cutoff).await.unwrap();

    let after = count_in_series(&h.pool, created.series.id).await;
    assert_eq!(after, 5, "expected exactly 5 surviving occurrences");
    assert_eq!(deleted as i64, total_before - 5);

    // Surviving rows are exactly the first 5.
    let surviving: Vec<chrono::NaiveDateTime> = sqlx::query_scalar(
        "SELECT start_time FROM events WHERE series_id = ? ORDER BY start_time ASC",
    )
    .bind(created.series.id.to_string())
    .fetch_all(&h.pool).await.unwrap();
    for (i, naive) in surviving.iter().enumerate() {
        let dt = DateTime::<Utc>::from_naive_utc_and_offset(*naive, Utc);
        assert_eq!(dt, created.occurrences[i].start_time);
    }

    // Series row reflects the new cap.
    let series = h.series_repo.find_by_id(created.series.id).await.unwrap().unwrap();
    assert_eq!(series.until_date, Some(cutoff));
}

// --------------------------------------------------------------------
// Delete entire series (cascade should drop occurrences too)
// --------------------------------------------------------------------

#[tokio::test]
async fn delete_series_cascades_to_occurrences() {
    let h = build().await;
    let anchor = Utc.with_ymd_and_hms(2026, 5, 5, 18, 0, 0).unwrap();
    let rule = Recurrence::MonthlyByDayOfMonth { interval: 1, day: 5 };
    let created = h.service.create_series_with_initial_materialization(
        rule, template(h.creator, anchor),
        Some(Utc.with_ymd_and_hms(2026, 12, 1, 0, 0, 0).unwrap()),
        h.creator,
    ).await.unwrap();
    assert!(count_in_series(&h.pool, created.series.id).await > 0);

    h.series_repo.delete(created.series.id).await.unwrap();
    assert_eq!(count_in_series(&h.pool, created.series.id).await, 0);
    assert!(h.series_repo.find_by_id(created.series.id).await.unwrap().is_none());
}

// --------------------------------------------------------------------
// Monthly-by-weekday smoke test (exercises a different rule kind
// through the materializer)
// --------------------------------------------------------------------

#[tokio::test]
async fn second_wednesday_materializes_correctly() {
    let h = build().await;
    let anchor = Utc.with_ymd_and_hms(2026, 5, 13, 19, 0, 0).unwrap(); // 2nd Wed of May 2026
    let rule = Recurrence::MonthlyByWeekdayOrdinal {
        interval: 1, weekday: WeekdayCode::Wed, ordinal: 2,
    };
    let until = Utc.with_ymd_and_hms(2026, 12, 31, 0, 0, 0).unwrap();
    let created = h.service.create_series_with_initial_materialization(
        rule, template(h.creator, anchor), Some(until), h.creator,
    ).await.unwrap();

    // May, Jun, Jul, Aug, Sep, Oct, Nov, Dec 2026 — 8 months.
    assert_eq!(created.occurrences.len(), 8);
    for occ in &created.occurrences {
        assert_eq!(occ.start_time.weekday(), Weekday::Wed);
        assert!((8..=14).contains(&occ.start_time.day()));
    }
}

// --------------------------------------------------------------------
// extend_horizon_for_active_series — bulk path
// --------------------------------------------------------------------

#[tokio::test]
async fn extend_horizon_for_active_series_processes_every_series() {
    let h = build().await;
    let anchor = Utc.with_ymd_and_hms(2026, 5, 5, 18, 0, 0).unwrap();

    // Two series with short caps, both opened back up via NULL until_date.
    for _ in 0..2 {
        let created = h.service.create_series_with_initial_materialization(
            Recurrence::WeeklyByDay {
                interval: 1, weekdays: vec![WeekdayCode::Tue],
            },
            template(h.creator, anchor),
            Some(Utc.with_ymd_and_hms(2026, 6, 30, 0, 0, 0).unwrap()),
            h.creator,
        ).await.unwrap();
        sqlx::query("UPDATE event_series SET until_date = NULL WHERE id = ?")
            .bind(created.series.id.to_string())
            .execute(&h.pool).await.unwrap();
    }

    let total_added = h.service
        .extend_horizon_for_active_series().await.unwrap();
    assert!(total_added > 0,
        "bulk run must materialize some new occurrences");
}
