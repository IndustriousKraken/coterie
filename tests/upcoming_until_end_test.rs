//! Integration tests for a52: an event stays upcoming until it ENDS.
//!
//! The three hand-written `start > now` copies (`list_upcoming`,
//! `count_members_only_upcoming`, and the `/public/events` default
//! branch) now read one domain predicate, so an event in progress stays
//! in the portal list, in the members-only teaser count, on the
//! marketing feed, and in every calendar subscription.
//!
//! The boundary math itself is unit-tested in `src/domain/event.rs`;
//! what needs a real DB + router lives here.
//!
//! Run: cargo test --test upcoming_until_end_test

use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use chrono::{DateTime, Duration, Utc};
use coterie::repository::{EventRepository, SqliteEventRepository};
use sqlx::SqlitePool;
use tower::ServiceExt;
use uuid::Uuid;

mod common;
use common::{build_app_state, fresh_pool, make_member};

/// Insert an event whose stored wall-clock is `start`/`end` in `zone`.
///
/// The caller passes wall-clocks, because that is what the column holds
/// — for a UTC zone the wall-clock and the instant coincide, which is
/// what makes the repository fixtures below relative-to-`now` and
/// deterministic at once.
#[allow(clippy::too_many_arguments)]
async fn insert_event(
    pool: &SqlitePool,
    created_by: Uuid,
    visibility: &str,
    title: &str,
    zone: &str,
    start: DateTime<Utc>,
    end: Option<DateTime<Utc>>,
) -> Uuid {
    let id = Uuid::new_v4();
    let fmt = |t: DateTime<Utc>| t.format("%Y-%m-%d %H:%M:%S").to_string();
    sqlx::query(
        "INSERT INTO events \
           (id, title, description, event_type, visibility, start_time, end_time, timezone, \
            location, max_attendees, rsvp_required, created_by) \
         VALUES (?, ?, 'Real description', 'Social', ?, ?, ?, ?, 'Room 5', 100, 1, ?)",
    )
    .bind(id.to_string())
    .bind(title)
    .bind(visibility)
    .bind(fmt(start))
    .bind(end.map(fmt))
    .bind(zone)
    .bind(created_by.to_string())
    .execute(pool)
    .await
    .expect("insert event");
    id
}

fn hours(n: i64) -> Duration {
    Duration::hours(n)
}

async fn get(app: Router, uri: &str) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    (status, headers, bytes)
}

fn titles(body: &[u8]) -> Vec<String> {
    serde_json::from_slice::<serde_json::Value>(body)
        .expect("json array")
        .as_array()
        .expect("array response")
        .iter()
        .map(|e| e["title"].as_str().unwrap_or_default().to_string())
        .collect()
}

// 3.2 — the listing keeps an event that has started but not ended, and
// drops one that ended a minute ago. This is the behavior change.
#[tokio::test]
async fn list_upcoming_keeps_in_progress_and_drops_just_ended() {
    let pool = fresh_pool().await;
    let actor = make_member(&pool).await;
    let now = Utc::now();

    let in_progress = insert_event(
        &pool,
        actor,
        "Public",
        "In Progress",
        "UTC",
        now - hours(1),
        Some(now + hours(1)),
    )
    .await;
    let just_ended = insert_event(
        &pool,
        actor,
        "Public",
        "Just Ended",
        "UTC",
        now - hours(2),
        Some(now - Duration::minutes(1)),
    )
    .await;

    let listed = SqliteEventRepository::new(pool.clone())
        .list_upcoming(50)
        .await
        .unwrap();
    let ids: Vec<Uuid> = listed.iter().map(|e| e.id).collect();

    assert!(
        ids.contains(&in_progress),
        "an event that started an hour ago and ends in an hour is still upcoming: {ids:?}",
    );
    assert!(
        !ids.contains(&just_ended),
        "an event that ended a minute ago is not: {ids:?}",
    );
}

// 3.3 — an in-progress event sorts by its start like any other, which
// puts it at the head of the ascending list rather than in a section of
// its own.
#[tokio::test]
async fn in_progress_sorts_ahead_of_later_today() {
    let pool = fresh_pool().await;
    let actor = make_member(&pool).await;
    let now = Utc::now();

    insert_event(
        &pool,
        actor,
        "Public",
        "Later Today",
        "UTC",
        now + hours(3),
        Some(now + hours(4)),
    )
    .await;
    insert_event(
        &pool,
        actor,
        "Public",
        "In Progress",
        "UTC",
        now - hours(1),
        Some(now + hours(1)),
    )
    .await;

    let listed = SqliteEventRepository::new(pool.clone())
        .list_upcoming(50)
        .await
        .unwrap();
    let order: Vec<&str> = listed.iter().map(|e| e.title.as_str()).collect();
    assert_eq!(
        order,
        vec!["In Progress", "Later Today"],
        "the event happening now belongs at the head of the list",
    );
}

// 3.4 — a missing end time means UNKNOWN, so the two-hour grace decides:
// 30 minutes in is still upcoming, 3 hours in is not.
#[tokio::test]
async fn missing_end_time_falls_back_to_the_grace_period() {
    let pool = fresh_pool().await;
    let actor = make_member(&pool).await;
    let now = Utc::now();

    let inside = insert_event(
        &pool,
        actor,
        "Public",
        "Started 30m Ago",
        "UTC",
        now - Duration::minutes(30),
        None,
    )
    .await;
    let expired = insert_event(
        &pool,
        actor,
        "Public",
        "Started 3h Ago",
        "UTC",
        now - hours(3),
        None,
    )
    .await;

    let listed = SqliteEventRepository::new(pool.clone())
        .list_upcoming(50)
        .await
        .unwrap();
    let ids: Vec<Uuid> = listed.iter().map(|e| e.id).collect();

    assert!(
        ids.contains(&inside),
        "no end time, started 30 minutes ago — inside the grace period: {ids:?}",
    );
    assert!(
        !ids.contains(&expired),
        "no end time, started 3 hours ago — the grace period has run out: {ids:?}",
    );
}

// 3.5 — the teaser count and the list it teases must agree, including
// while one of the members-only events is in progress. This is the
// assertion that stops the two implementations drifting again.
#[tokio::test]
async fn members_only_count_matches_the_list_it_teases() {
    let pool = fresh_pool().await;
    let actor = make_member(&pool).await;
    let now = Utc::now();

    // A deliberately awkward fixture: in progress, future, just ended,
    // no-end-inside-grace, no-end-expired — plus public noise the count
    // must not pick up.
    let cases = [
        (
            "MembersOnly",
            "MO In Progress",
            now - hours(1),
            Some(now + hours(1)),
        ),
        (
            "MembersOnly",
            "MO Future",
            now + hours(30),
            Some(now + hours(31)),
        ),
        (
            "MembersOnly",
            "MO Ended",
            now - hours(4),
            Some(now - hours(3)),
        ),
        (
            "MembersOnly",
            "MO No End Fresh",
            now - Duration::minutes(15),
            None,
        ),
        ("MembersOnly", "MO No End Stale", now - hours(5), None),
        (
            "Public",
            "Public In Progress",
            now - hours(1),
            Some(now + hours(1)),
        ),
    ];
    for (visibility, title, start, end) in cases {
        insert_event(&pool, actor, visibility, title, "UTC", start, end).await;
    }

    let repo = SqliteEventRepository::new(pool.clone());
    let listed_members_only = repo
        .list_upcoming(1000)
        .await
        .unwrap()
        .into_iter()
        .filter(|e| format!("{:?}", e.visibility) == "MembersOnly")
        .count() as i64;
    let counted = repo.count_members_only_upcoming().await.unwrap();

    assert_eq!(
        counted, listed_members_only,
        "the teaser count must equal the members-only rows list_upcoming yields",
    );
    assert_eq!(counted, 3, "in progress + future + no-end-inside-grace");
}

// 3.6 — the public feed keeps an in-progress event, in JSON and in the
// iCal a calendar client subscribes to. The org zone is deliberately NOT
// UTC: a UTC fixture cannot detect a double conversion, because the
// offset it would add is zero, and the bug would ship green.
#[tokio::test]
async fn public_feed_keeps_an_in_progress_event_in_a_non_utc_zone() {
    let pool = fresh_pool().await;
    let actor = make_member(&pool).await;
    // America/Phoenix is UTC-7 year round — no DST, so the offset the
    // fixture relies on is fixed.
    let zone = "America/Phoenix";
    let offset = hours(7);
    let now = Utc::now();

    // Stored wall-clock = instant - 7h, so these derive to "started an
    // hour ago, ends in an hour" and "ended an hour ago".
    insert_event(
        &pool,
        actor,
        "Public",
        "Phoenix In Progress",
        zone,
        now - hours(1) - offset,
        Some(now + hours(1) - offset),
    )
    .await;
    insert_event(
        &pool,
        actor,
        "Public",
        "Phoenix Ended",
        zone,
        now - hours(3) - offset,
        Some(now - hours(1) - offset),
    )
    .await;

    let app = coterie::api::create_app(build_app_state(pool.clone()).await);
    let (status, _h, body) = get(app, "/public/events").await;
    assert_eq!(status, StatusCode::OK);
    let t = titles(&body);
    assert!(
        t.contains(&"Phoenix In Progress".to_string()),
        "an event in progress stays on the marketing feed: {t:?}",
    );
    assert!(
        !t.contains(&"Phoenix Ended".to_string()),
        "an event that has ended does not: {t:?}",
    );

    let app = coterie::api::create_app(build_app_state(pool.clone()).await);
    let (status, headers, body) = get(app, "/public/events?format=ical").await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .starts_with("text/calendar"));
    let ics = String::from_utf8(body).unwrap();
    assert!(
        ics.contains("SUMMARY:Phoenix In Progress"),
        "the VEVENT for an event in progress stays in the subscribed feed: {ics}",
    );
    assert!(
        !ics.contains("SUMMARY:Phoenix Ended"),
        "an ended event still drops out of the feed: {ics}",
    );
}

// 3.8 — past events remain excluded by default, and the `from`/`to`
// range remains the way to reach them. An event that ended an hour ago
// is the interesting case: under the old start-based rule it and an
// in-progress event were indistinguishable.
#[tokio::test]
async fn ended_event_is_excluded_by_default_and_returned_by_a_range() {
    let pool = fresh_pool().await;
    let actor = make_member(&pool).await;
    let now = Utc::now();

    insert_event(
        &pool,
        actor,
        "Public",
        "Ended An Hour Ago",
        "UTC",
        now - hours(3),
        Some(now - hours(1)),
    )
    .await;

    let app = coterie::api::create_app(build_app_state(pool.clone()).await);
    let (status, _h, body) = get(app, "/public/events").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !titles(&body).contains(&"Ended An Hour Ago".to_string()),
        "an event that has ended is gone from the default feed",
    );

    let from = (now - hours(24)).to_rfc3339();
    let to = (now + hours(24)).to_rfc3339();
    let app = coterie::api::create_app(build_app_state(pool.clone()).await);
    let (status, _h, body) = get(
        app,
        &format!(
            "/public/events?from={}&to={}",
            urlencoding(&from),
            urlencoding(&to)
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        titles(&body).contains(&"Ended An Hour Ago".to_string()),
        "the range is still the way to see events that have ended",
    );
}

/// Percent-encode the `+` in an RFC 3339 offset so the query parser
/// doesn't read it as a space. Only the characters an instant contains.
fn urlencoding(s: &str) -> String {
    s.replace('+', "%2B")
}

// 3.10 — the defect class is a FOURTH copy of the rule. Every surface
// outside the domain now asks `is_upcoming`; anything still comparing a
// start instant to `now` is either one of the three deliberate
// exceptions below or a regression.
#[test]
fn no_call_site_outside_the_domain_re_derives_upcoming_ness() {
    // Files that legitimately compare a START to `now`, and why:
    //   - series_enrollment_service: whether a class can still be BOUGHT
    //     (2.4), plus the seating/refund paths that materialize
    //     attendance only for sessions that have not begun.
    //   - class_register: counts the sessions a buyer would still
    //     receive (2.5) — a pricing question, not a listing one.
    //   - admin/events/occurrences: the cancel/override controls, which
    //     `admin-events` canon fixes at `start_time < now` because
    //     exceptions only apply to the future (2.7).
    //   - domain/event: the predicate's own home and its unit tests.
    const ALLOWED: &[&str] = &[
        "service/series_enrollment_service.rs",
        "web/templates/class_register.rs",
        "web/portal/admin/events/occurrences.rs",
        "domain/event.rs",
    ];

    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    for path in rust_files(&src) {
        let rel = path
            .strip_prefix(&src)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if ALLOWED.contains(&rel.as_str()) {
            continue;
        }
        let body = std::fs::read_to_string(&path).expect("read source");
        for (i, line) in body.lines().enumerate() {
            if compares_start_to_now(line) {
                offenders.push(format!("src/{rel}:{}: {}", i + 1, line.trim()));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "these compare a start instant to `now` instead of calling \
         `is_upcoming` — add the call site to the domain predicate, or to \
         ALLOWED with a reason:\n{}",
        offenders.join("\n"),
    );
}

/// True when `line` compares an event's start against `now` — the shape
/// this change replaced. Comments are stripped first so the prose
/// explaining the old rule doesn't trip the assertion.
fn compares_start_to_now(line: &str) -> bool {
    let code = line.split("//").next().unwrap_or(line);
    [
        "start_utc() >",
        "start_utc() <",
        "start_time >",
        "start_time <",
    ]
    .iter()
    .any(|op| {
        code.split(op).skip(1).any(|rest| {
            rest.trim_start()
                .trim_start_matches('=')
                .trim_start()
                .starts_with("now")
        })
    })
}

fn rust_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).expect("read_dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    out
}
