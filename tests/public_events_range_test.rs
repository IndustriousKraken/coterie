//! Integration tests for the public-events-past-via-range change.
//!
//! `GET /public/events` is upcoming-only by default, but an explicit,
//! valid, bounded `from`/`to` range opts the JSON feed into returning
//! events in `[from, to)` — INCLUDING past events — still projected and
//! members-only-sanitized. A malformed / over-wide range, and every iCal
//! request, fall back to the unchanged upcoming-only behavior.
//!
//! Run: cargo test --test public_events_range_test

use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use sqlx::SqlitePool;
use tower::ServiceExt;
use uuid::Uuid;

mod common;
use common::{build_app_state, fresh_pool, make_member};

/// A past event (June 2020) and a far-future event (2099), both in the
/// `UTC` zone so the derived instant equals the stored wall-clock. Titles
/// let the assertions target a specific row.
async fn insert_event(
    pool: &SqlitePool,
    created_by: Uuid,
    visibility: &str,
    title: &str,
    start: &str,
    end: &str,
) {
    sqlx::query(
        "INSERT INTO events \
           (id, title, description, event_type, visibility, start_time, end_time, timezone, \
            location, max_attendees, rsvp_required, image_url, created_by) \
         VALUES (?, ?, 'Real description', 'Social', ?, ?, ?, 'UTC', \
                 'Secret Room 5', 100, 1, 'https://example.com/img.png', ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(title)
    .bind(visibility)
    .bind(start)
    .bind(end)
    .bind(created_by.to_string())
    .execute(pool)
    .await
    .expect("insert event");
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

fn titles(json: &serde_json::Value) -> Vec<String> {
    json.as_array()
        .expect("array response")
        .iter()
        .map(|e| e["title"].as_str().unwrap_or_default().to_string())
        .collect()
}

// 2.1 — a range spanning a past month returns the past event, correctly
// projected; the members-only one is sanitized like any other.
#[tokio::test]
async fn range_returns_past_events_projected_and_sanitized() {
    let pool = fresh_pool().await;
    let actor = make_member(&pool).await;
    insert_event(
        &pool,
        actor,
        "Public",
        "Past Public",
        "2020-06-15 19:00:00",
        "2020-06-15 21:00:00",
    )
    .await;
    insert_event(
        &pool,
        actor,
        "MembersOnly",
        "Past Members",
        "2020-06-20 19:00:00",
        "2020-06-20 21:00:00",
    )
    .await;

    let app = coterie::api::create_app(build_app_state(pool.clone()).await);
    let (status, _h, body) = get(
        app,
        "/public/events?from=2020-06-01T00:00:00Z&to=2020-07-01T00:00:00Z",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let events = json.as_array().expect("array");
    assert_eq!(events.len(), 2, "both past events in range: {events:?}");

    let public = events
        .iter()
        .find(|e| e["visibility"] == "Public")
        .expect("public event present");
    assert_eq!(public["title"], "Past Public");
    // Projection still strips internal identifiers for a past event.
    for field in ["created_by", "created_at", "event_type_id", "series_id"] {
        assert!(
            !public.as_object().unwrap().contains_key(field),
            "internal field `{field}` leaked in range result",
        );
    }

    let members = events
        .iter()
        .find(|e| e["visibility"] == "MembersOnly")
        .expect("members-only event present");
    assert_eq!(members["title"], "Members-Only Event");
    assert!(members["location"].is_null(), "location sanitized");
    assert!(members["image_url"].is_null(), "image_url sanitized");
}

// 2.2 — with no range, a past event is still excluded (unchanged default).
#[tokio::test]
async fn no_range_still_excludes_past() {
    let pool = fresh_pool().await;
    let actor = make_member(&pool).await;
    insert_event(
        &pool,
        actor,
        "Public",
        "Past Public",
        "2020-06-15 19:00:00",
        "2020-06-15 21:00:00",
    )
    .await;
    insert_event(
        &pool,
        actor,
        "Public",
        "Future Public",
        "2099-06-15 19:00:00",
        "2099-06-15 21:00:00",
    )
    .await;

    let app = coterie::api::create_app(build_app_state(pool.clone()).await);
    let (status, _h, body) = get(app, "/public/events").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let t = titles(&json);
    assert!(t.contains(&"Future Public".to_string()), "future kept: {t:?}");
    assert!(!t.contains(&"Past Public".to_string()), "past excluded: {t:?}");
}

// 2.3 — a malformed (from only), unparseable, or over-wide (> MAX_SPAN)
// range falls back to the upcoming-only list, HTTP 200 (never an error).
#[tokio::test]
async fn bad_range_falls_back_to_upcoming() {
    let pool = fresh_pool().await;
    let actor = make_member(&pool).await;
    insert_event(
        &pool,
        actor,
        "Public",
        "Past Public",
        "2020-06-15 19:00:00",
        "2020-06-15 21:00:00",
    )
    .await;
    insert_event(
        &pool,
        actor,
        "Public",
        "Future Public",
        "2099-06-15 19:00:00",
        "2099-06-15 21:00:00",
    )
    .await;

    let cases = [
        // from present, to missing
        "/public/events?from=2020-06-01T00:00:00Z",
        // unparseable
        "/public/events?from=not-a-date&to=also-bad",
        // over-wide: ~547 days > 400-day cap
        "/public/events?from=2020-01-01T00:00:00Z&to=2021-07-01T00:00:00Z",
        // inverted (to <= from)
        "/public/events?from=2020-07-01T00:00:00Z&to=2020-06-01T00:00:00Z",
    ];
    for uri in cases {
        let app = coterie::api::create_app(build_app_state(pool.clone()).await);
        let (status, _h, body) = get(app, uri).await;
        assert_eq!(status, StatusCode::OK, "bad range must be 200: {uri}");
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let t = titles(&json);
        assert!(
            t.contains(&"Future Public".to_string()) && !t.contains(&"Past Public".to_string()),
            "bad range `{uri}` must fall back to upcoming-only, got {t:?}",
        );
    }
}

// 2.4 — format=ical ignores the range: still upcoming-only, so a past
// event in the requested window does not appear in the .ics feed.
#[tokio::test]
async fn ical_ignores_range() {
    let pool = fresh_pool().await;
    let actor = make_member(&pool).await;
    insert_event(
        &pool,
        actor,
        "Public",
        "Past Public",
        "2020-06-15 19:00:00",
        "2020-06-15 21:00:00",
    )
    .await;

    let app = coterie::api::create_app(build_app_state(pool.clone()).await);
    let (status, headers, body) = get(
        app,
        "/public/events?format=ical&from=2020-06-01T00:00:00Z&to=2020-07-01T00:00:00Z",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .starts_with("text/calendar"));
    let ics = String::from_utf8(body).unwrap();
    assert!(
        !ics.contains("SUMMARY:Past Public"),
        "iCal must ignore the range and stay upcoming-only: {ics}",
    );
}
