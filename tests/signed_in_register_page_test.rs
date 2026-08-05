//! a51: the shareable registration pages recognize a signed-in member.
//!
//! `GET /events/:id/register` and `GET /classes/:id/register` are the
//! URLs an organizer pastes into Discord. A member who is already signed
//! in used to be handed the guest form and charged the guest price —
//! a real Stripe charge on a guest attendee row, undoable only by refund
//! or by an admin re-seating them. These tests pin both renderings: the
//! member gets the authenticated action, the guest gets exactly today's
//! page.
//!
//! They run against the FULL merged app (api ∪ web, setup gate, CSRF)
//! because what is under test is a cookie reaching a route on the
//! anonymous web surface.
//!
//! Run with: cargo test --test signed_in_register_page_test

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use chrono::{Duration, Utc};
use coterie::{
    api::state::AppState,
    domain::{
        Attendee, Event, EventSeries, EventType, EventVisibility, MemberStatus,
        UpdateMemberRequest, UpdateSettingRequest,
    },
    repository::{
        EventRepository, EventSeriesRepository, MemberRepository, SeriesEnrollmentRepository,
        SqliteEventRepository, SqliteEventSeriesRepository, SqliteMemberRepository,
        SqliteSeriesEnrollmentRepository,
    },
    service::settings_service::bot_challenge_keys,
};
use sqlx::SqlitePool;
use tower::ServiceExt;
use uuid::Uuid;

mod common;
use common::{build_app_state_full, fresh_pool, make_member, merged_router};

/// Merged app plus a configured bot-challenge provider, so "the guest
/// page still renders the challenge" and "the member page does not" are
/// both observable.
async fn build(pool: &SqlitePool) -> (Router, AppState) {
    let admin = make_member(pool).await;
    sqlx::query("UPDATE members SET is_admin = 1, status = 'Active' WHERE id = ?")
        .bind(admin.to_string())
        .execute(pool)
        .await
        .unwrap();

    let state = build_app_state_full(pool.clone(), None, None, None).await;
    for (key, value) in [
        (bot_challenge_keys::PROVIDER, "turnstile"),
        (bot_challenge_keys::SITE_KEY, SITE_KEY),
    ] {
        state
            .service_context
            .settings_service
            .update_setting(
                key,
                UpdateSettingRequest {
                    value: value.to_string(),
                    reason: None,
                },
                admin,
            )
            .await
            .expect("configure bot challenge");
    }

    (merged_router(state.clone()), state)
}

const SITE_KEY: &str = "0x4AAAAAAATestSiteKey";

/// An Active member and a `session=` cookie header for them.
async fn signed_in_member(pool: &SqlitePool, state: &AppState) -> (Uuid, String) {
    let member_id = make_member(pool).await;
    let repo: Arc<dyn MemberRepository> = Arc::new(SqliteMemberRepository::new(pool.clone()));
    repo.update(
        member_id,
        UpdateMemberRequest {
            status: Some(MemberStatus::Active),
            ..Default::default()
        },
    )
    .await
    .expect("activate member");

    let (_session, token) = state
        .service_context
        .auth_service
        .create_session(member_id, 24)
        .await
        .expect("create session");
    (member_id, format!("session={}", token))
}

async fn make_event(
    pool: &SqlitePool,
    visibility: EventVisibility,
    member_price_cents: i64,
    guest_price_cents: i64,
    guest_registration_enabled: bool,
) -> Event {
    let repo = SqliteEventRepository::new(pool.clone());
    let creator = make_member(pool).await;
    let now = Utc::now();
    repo.create(Event {
        id: Uuid::new_v4(),
        title: "Lockpicking 101".to_string(),
        description: "Bring a padlock".to_string(),
        event_type: EventType::Workshop,
        event_type_id: None,
        visibility,
        start_time: now + Duration::days(7),
        end_time: None,
        timezone: "UTC".to_string(),
        location: Some("The Shop".to_string()),
        max_attendees: Some(20),
        rsvp_required: true,
        member_price_cents,
        guest_price_cents,
        guest_registration_enabled,
        image_url: None,
        created_by: creator,
        created_at: now,
        updated_at: now,
        series_id: None,
        occurrence_index: None,
    })
    .await
    .unwrap()
}

/// A publicly enrollable class: a series with one `Public` occurrence.
async fn make_class(
    pool: &SqlitePool,
    member_price_cents: i64,
    guest_price_cents: i64,
) -> EventSeries {
    let series_repo = SqliteEventSeriesRepository::new(pool.clone());
    let event_repo = SqliteEventRepository::new(pool.clone());
    let creator = make_member(pool).await;
    let now = Utc::now();
    let series = series_repo
        .create(EventSeries {
            id: Uuid::new_v4(),
            rule_kind: "weekly_by_day".to_string(),
            rule_json: r#"{"kind":"weekly_by_day","interval":1,"weekdays":["tue"]}"#.to_string(),
            until_date: Some(now + Duration::days(30)),
            materialized_through: now + Duration::days(30),
            member_price_cents,
            guest_price_cents,
            guest_registration_enabled: true,
            max_enrollments: Some(12),
            created_by: creator,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    event_repo
        .create(Event {
            id: Uuid::new_v4(),
            title: "Intro to Lockpicking".to_string(),
            description: "Six Tuesdays".to_string(),
            event_type: EventType::Workshop,
            event_type_id: None,
            visibility: EventVisibility::Public,
            start_time: now + Duration::days(7),
            end_time: None,
            timezone: "UTC".to_string(),
            location: None,
            max_attendees: None,
            rsvp_required: true,
            member_price_cents: 0,
            guest_price_cents: 0,
            guest_registration_enabled: false,
            image_url: None,
            created_by: creator,
            created_at: now,
            updated_at: now,
            series_id: Some(series.id),
            occurrence_index: Some(1),
        })
        .await
        .unwrap();
    series
}

/// GET `uri`, optionally carrying a cookie header.
async fn get(app: &Router, uri: &str, cookie: Option<&str>) -> (StatusCode, String) {
    let mut req = Request::builder().uri(uri);
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    let resp = app
        .clone()
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

/// The guest form's two inputs and the bot challenge: what a
/// session-authenticated member must never be shown.
fn assert_no_guest_form(body: &str) {
    assert!(
        !body.contains(r#"name="name""#) && !body.contains(r#"name="email""#),
        "a signed-in member is not asked for details already on file: {body}",
    );
    assert!(
        !body.contains(SITE_KEY) && !body.contains("cf-turnstile"),
        "the authenticated path is not an anonymous money endpoint: {body}",
    );
    assert!(
        !body.contains("/public/events/") && !body.contains("/public/series/"),
        "the guest endpoint must not be offered to a signed-in member: {body}",
    );
}

// ---------------------------------------------------------------------
// 5.1 / 5.2 / 5.8 — the two renderings of the event page
// ---------------------------------------------------------------------

#[tokio::test]
async fn signed_in_member_is_offered_the_member_price_and_the_authenticated_action() {
    let pool = fresh_pool().await;
    let (app, state) = build(&pool).await;
    let (_member, cookie) = signed_in_member(&pool, &state).await;
    // Members pay $10, guests pay $30 — the silent overcharge this fixes.
    let event = make_event(&pool, EventVisibility::Public, 1000, 3000, true).await;
    let uri = format!("/events/{}/register", event.id);

    let (status, body) = get(&app, &uri, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("$10.00"), "the member price is shown: {body}");
    assert!(
        !body.contains("$30.00"),
        "the guest price they are not paying is not shown: {body}",
    );
    assert!(
        body.contains(&format!(
            r#"hx-post="/portal/api/events/{}/rsvp""#,
            event.id
        )),
        "the authenticated action is the one the portal uses: {body}",
    );
    assert!(
        !body.contains(r#"<meta name="csrf-token" content="">"#),
        "the action is CSRF-protected, so the page must carry a real token: {body}",
    );
    assert_no_guest_form(&body);

    // 5.2 / 5.8 — the anonymous rendering is unchanged, and still
    // reaches the guest endpoint.
    let (status, body) = get(&app, &uri, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("$30.00"), "the guest price is shown: {body}");
    assert!(
        body.contains(&format!("/public/events/{}/register", event.id)),
        "the guest form still posts to the guest endpoint: {body}",
    );
    assert!(body.contains(r#"name="name""#) && body.contains(r#"name="email""#));
    assert!(body.contains(SITE_KEY), "the bot challenge is rendered");
    assert!(
        body.contains("Members pay $10.00")
            && body.contains(&format!(
                "/login?redirect=%2Fevents%2F{}%2Fregister",
                event.id
            )),
        "the login link carries a return path to this page: {body}",
    );
}

// ---------------------------------------------------------------------
// 5.3 — session resolution fails open
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_session_that_does_not_validate_renders_the_guest_page() {
    let pool = fresh_pool().await;
    let (app, state) = build(&pool).await;
    let event = make_event(&pool, EventVisibility::Public, 1000, 3000, true).await;
    let uri = format!("/events/{}/register", event.id);

    let (_status, anonymous) = get(&app, &uri, None).await;

    // A syntactically bogus cookie value.
    let (status, body) = get(&app, &uri, Some("session=not-a-real-token")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body, anonymous,
        "a garbage cookie must render exactly the anonymous page",
    );

    // A session row that has expired.
    let (member_id, cookie) = signed_in_member(&pool, &state).await;
    sqlx::query("UPDATE sessions SET expires_at = ? WHERE member_id = ?")
        .bind((Utc::now() - Duration::days(1)).naive_utc())
        .bind(member_id.to_string())
        .execute(&pool)
        .await
        .unwrap();

    let (status, body) = get(&app, &uri, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body, anonymous,
        "an expired session must render exactly the anonymous page",
    );
    assert!(
        body.contains(SITE_KEY),
        "including the bot challenge: {body}",
    );
}

// ---------------------------------------------------------------------
// 5.4 — already holding a seat
// ---------------------------------------------------------------------

#[tokio::test]
async fn signed_in_member_who_already_has_a_seat_is_told_so() {
    let pool = fresh_pool().await;
    let (app, state) = build(&pool).await;
    let (member_id, cookie) = signed_in_member(&pool, &state).await;
    let event = make_event(&pool, EventVisibility::Public, 1000, 3000, true).await;

    let repo: Arc<dyn EventRepository> = Arc::new(SqliteEventRepository::new(pool.clone()));
    repo.register_attendance(event.id, &Attendee::Member(member_id))
        .await
        .unwrap();

    let (status, body) = get(
        &app,
        &format!("/events/{}/register", event.id),
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("You're already registered"),
        "the page says they already hold a seat: {body}",
    );
    assert!(
        !body.contains("/rsvp") && !body.contains("<form"),
        "no action that looks like it will charge them again: {body}",
    );
    assert_no_guest_form(&body);
}

// ---------------------------------------------------------------------
// 5.5 — a session does not widen what is visible
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_session_does_not_make_a_non_registerable_event_visible() {
    let pool = fresh_pool().await;
    let (app, state) = build(&pool).await;
    let (_member, cookie) = signed_in_member(&pool, &state).await;

    // Members-only, and a public event with guest registration off:
    // neither is publicly registerable.
    let private = make_event(&pool, EventVisibility::MembersOnly, 1000, 3000, true).await;
    let show_up = make_event(&pool, EventVisibility::Public, 0, 0, false).await;

    for id in [private.id, show_up.id, Uuid::new_v4()] {
        let uri = format!("/events/{}/register", id);
        let (signed_in_status, signed_in_body) = get(&app, &uri, Some(&cookie)).await;
        let (anon_status, anon_body) = get(&app, &uri, None).await;
        assert_eq!(signed_in_status, StatusCode::NOT_FOUND, "id {id}");
        assert_eq!(
            (signed_in_status, signed_in_body),
            (anon_status, anon_body),
            "a session must not change the answer for {id}",
        );
    }
}

// ---------------------------------------------------------------------
// 4.3 — a live session arriving at /login is not bounced to the dashboard
// ---------------------------------------------------------------------

#[tokio::test]
async fn login_page_honors_an_allow_listed_return_path_for_a_live_session() {
    let pool = fresh_pool().await;
    let (app, state) = build(&pool).await;
    let (_member, cookie) = signed_in_member(&pool, &state).await;
    let event = make_event(&pool, EventVisibility::Public, 1000, 3000, true).await;

    let location = |uri: String| {
        let app = app.clone();
        let cookie = cookie.clone();
        async move {
            let resp = app
                .oneshot(
                    Request::builder()
                        .uri(uri)
                        .header(header::COOKIE, cookie)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            resp.headers()
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string()
        }
    };

    let dest = format!("/events/{}/register", event.id);
    assert_eq!(
        location(format!("/login?redirect={}", urlencoding::encode(&dest))).await,
        dest,
        "a member who arrives with a live session lands on the event they opened",
    );
    assert_eq!(
        location("/login?redirect=https%3A%2F%2Fevil.example%2F".to_string()).await,
        "/portal/dashboard",
        "an off-site destination falls back to the default",
    );
    assert_eq!(
        location("/login".to_string()).await,
        "/portal/dashboard",
        "no destination is still the dashboard",
    );
}

// ---------------------------------------------------------------------
// 5.6 — the class page, at series scope
// ---------------------------------------------------------------------

#[tokio::test]
async fn the_class_page_recognizes_the_session_the_same_way() {
    let pool = fresh_pool().await;
    let (app, state) = build(&pool).await;
    let (member_id, cookie) = signed_in_member(&pool, &state).await;
    let series = make_class(&pool, 4000, 6000).await;
    let uri = format!("/classes/{}/register", series.id);

    let (status, body) = get(&app, &uri, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("$40.00"), "the member pass price: {body}");
    assert!(!body.contains("$60.00"), "not the guest price: {body}");
    assert!(
        body.contains(&format!(
            r#"hx-post="/portal/api/series/{}/enroll""#,
            series.id
        )),
        "the authenticated enrollment action: {body}",
    );
    assert_no_guest_form(&body);

    // Anonymous still gets the guest form and the return-path login link.
    let (status, body) = get(&app, &uri, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(&format!("/public/series/{}/enroll", series.id)));
    assert!(body.contains(&format!(
        "/login?redirect=%2Fclasses%2F{}%2Fregister",
        series.id
    )));

    // Already enrolled: the no-action panel.
    let enrollments: Arc<dyn SeriesEnrollmentRepository> =
        Arc::new(SqliteSeriesEnrollmentRepository::new(pool.clone()));
    enrollments
        .register(series.id, &Attendee::Member(member_id))
        .await
        .unwrap();

    let (status, body) = get(&app, &uri, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("You're already enrolled"),
        "the page says they already hold a pass: {body}",
    );
    assert!(
        !body.contains("/enroll") && !body.contains("<form"),
        "no action that looks like a second charge: {body}",
    );
}
