//! Integration tests for the public-events-omit-internal-fields change.
//!
//! `GET /public/events` (JSON) must return a PUBLIC PROJECTION, never the
//! raw `Event` struct: internal identifiers — `created_by` (the
//! organizer's member id), `created_at`, `updated_at`, `event_type_id`,
//! `series_id`, `occurrence_index` — must not reach anonymous callers, for
//! either a public or a members-only event. Members-only sanitization
//! (title/description replaced, location/image_url nulled) still applies.
//!
//! Run: cargo test --test public_events_projection_test

use std::collections::BTreeSet;

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

/// Internal fields canon says must be omitted for every event.
const INTERNAL_FIELDS: &[&str] = &[
    "created_by",
    "created_at",
    "updated_at",
    "event_type_id",
    "series_id",
    "occurrence_index",
];

/// The CLOSED field list enumerated by the `public-content-feeds`
/// requirement "Members-only events appear in /public/events with
/// sanitized fields". Canon says the projection SHALL expose only these,
/// so this is a transcription of canon and not a fixture to update when a
/// field is added — amend the requirement first.
const PROJECTION_FIELDS: &[&str] = &[
    "id",
    "title",
    "description",
    "description_html",
    "event_type",
    "visibility",
    "start_time",
    "end_time",
    "timezone",
    "location",
    "image_url",
    "max_attendees",
    "rsvp_required",
    "registration_url",
    "guest_price_cents",
];

async fn get_events(app: Router) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method("GET")
        .uri("/public/events")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

/// Insert one event with every internal field populated (real
/// `created_by`, timestamps via defaults) so the projection has something
/// to omit. `start_time` is a fixed far-future UTC wall-clock, so it
/// survives the upcoming filter regardless of when the test runs.
async fn insert_event(pool: &SqlitePool, created_by: Uuid, visibility: &str) {
    sqlx::query(
        "INSERT INTO events \
           (id, title, description, event_type, visibility, start_time, end_time, timezone, \
            location, max_attendees, rsvp_required, image_url, created_by) \
         VALUES (?, 'Launch Party', 'Real description', 'Social', ?, \
                 '2099-06-15 19:00:00', '2099-06-15 21:00:00', 'UTC', \
                 'Secret Room 5', 100, 1, 'https://example.com/img.png', ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(visibility)
    .bind(created_by.to_string())
    .execute(pool)
    .await
    .expect("insert event");
}

fn find_by_visibility<'a>(
    events: &'a [serde_json::Value],
    visibility: &str,
) -> &'a serde_json::Map<String, serde_json::Value> {
    events
        .iter()
        .find(|e| e["visibility"] == visibility)
        .unwrap_or_else(|| panic!("no {visibility} event in response"))
        .as_object()
        .expect("event is an object")
}

// 2.1 — a PUBLIC event's JSON omits all internal identifiers.
#[tokio::test]
async fn public_event_omits_internal_fields() {
    let pool = fresh_pool().await;
    let actor = make_member(&pool).await;
    insert_event(&pool, actor, "Public").await;

    let app = coterie::api::create_app(build_app_state(pool.clone()).await);
    let (status, json) = get_events(app).await;

    assert_eq!(status, StatusCode::OK);
    let events = json.as_array().expect("array response");
    let event = find_by_visibility(events, "Public");

    // Public details pass through untouched.
    assert_eq!(event["title"], "Launch Party");
    assert_eq!(event["location"], "Secret Room 5");
    assert_eq!(event["image_url"], "https://example.com/img.png");

    // No internal identifier reaches the anonymous caller — in
    // particular `created_by` (the organizer's member id).
    for field in INTERNAL_FIELDS {
        assert!(
            !event.contains_key(*field),
            "public event must omit internal field `{field}` \
             (in particular the organizer's member id)",
        );
    }
}

// 2.2 — a MEMBERS-ONLY event is sanitized AND still omits internals; the
// real start/end times pass through.
#[tokio::test]
async fn members_only_event_sanitized_and_omits_internal_fields() {
    let pool = fresh_pool().await;
    let actor = make_member(&pool).await;
    insert_event(&pool, actor, "MembersOnly").await;

    let app = coterie::api::create_app(build_app_state(pool.clone()).await);
    let (status, json) = get_events(app).await;

    assert_eq!(status, StatusCode::OK);
    let events = json.as_array().expect("array response");
    let event = find_by_visibility(events, "MembersOnly");

    // Sanitized display fields.
    assert_eq!(event["title"], "Members-Only Event");
    assert_eq!(
        event["description"],
        "This event is for members only. Log in to the portal to see details."
    );
    assert!(event["location"].is_null(), "location must be nulled");
    assert!(event["image_url"].is_null(), "image_url must be nulled");

    // Real start/end times pass through.
    assert!(
        event["start_time"]
            .as_str()
            .expect("start_time string")
            .starts_with("2099-06-15T19:00:00"),
        "real start_time must pass through, got {:?}",
        event["start_time"],
    );
    assert!(
        event["end_time"]
            .as_str()
            .expect("end_time string")
            .starts_with("2099-06-15T21:00:00"),
        "real end_time must pass through, got {:?}",
        event["end_time"],
    );

    // Internal identifiers still omitted for members-only events.
    for field in INTERNAL_FIELDS {
        assert!(
            !event.contains_key(*field),
            "members-only event must omit internal field `{field}`",
        );
    }
}

// a58 — the enumerated list is CLOSED, so the guard is set EQUALITY, not
// containment: it fails on a field added to `PublicEvent` and on a field
// removed from it. Enumerating internal fields (above) only catches the
// leaks someone thought to name — `description_html` was added to the
// struct, the list in canon was never amended, and nothing failed.
#[tokio::test]
async fn projection_key_set_equals_the_enumerated_field_list() {
    let pool = fresh_pool().await;
    let actor = make_member(&pool).await;
    insert_event(&pool, actor, "Public").await;
    insert_event(&pool, actor, "MembersOnly").await;

    let app = coterie::api::create_app(build_app_state(pool.clone()).await);
    let (status, json) = get_events(app).await;
    assert_eq!(status, StatusCode::OK);
    let events = json.as_array().expect("array response");

    let expected: BTreeSet<&str> = PROJECTION_FIELDS.iter().copied().collect();
    // Both visibilities: sanitization nulls values, it never drops keys.
    for visibility in ["Public", "MembersOnly"] {
        let event = find_by_visibility(events, visibility);
        let actual: BTreeSet<&str> = event.keys().map(String::as_str).collect();
        assert_eq!(
            actual,
            expected,
            "the {visibility} entry's key set must equal the field list enumerated by the \
             public-content-feeds requirement \"Members-only events appear in /public/events \
             with sanitized fields\" — that list is canon and closed, not an arbitrary \
             fixture. Unlisted keys present: {:?}. Enumerated keys missing: {:?}. Amend the \
             requirement, then this list.",
            actual.difference(&expected).collect::<Vec<_>>(),
            expected.difference(&actual).collect::<Vec<_>>(),
        );
    }
}
