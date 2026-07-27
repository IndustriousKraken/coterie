//! The public surface of guest event registration: the Coterie-hosted
//! page, the money endpoint, and the protections in front of it.
//!
//! These run against the FULL merged app the way `main.rs` builds it
//! (api ∪ web, setup check, top-level CSRF), because the things under
//! test here are routing-and-middleware properties: a 404 that must be
//! indistinguishable from another 404, a CSRF exemption on a
//! parameterized path, a rate limit that must fire before a provider is
//! consulted. A handler-level test would pass with any of those broken.
//!
//! Run with: cargo test --features test-utils --test guest_registration_test

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use chrono::{Duration, Utc};
use coterie::{
    api::middleware::bot_challenge::{test_utils::FakeVerifier, BotChallengeVerifier, VerifyError},
    domain::{AttendanceStatus, Attendee, Event, EventType, EventVisibility},
    payments::{
        fake_gateway::FakeStripeGateway, gateway::StripeGateway, StripeClient, StripeHandle,
    },
    repository::{
        EventRepository, MemberRepository, PaymentRepository, SqliteEventRepository,
        SqliteMemberRepository, SqlitePaymentRepository,
    },
};
use sqlx::SqlitePool;
use tower::ServiceExt;
use uuid::Uuid;

mod common;
use common::{build_app_state_full, fresh_pool, make_member};

const GUEST_FORM: &str = "name=Ada+Lovelace&email=ada%40example.com";
const GUEST_EMAIL: &str = "ada@example.com";

/// Build the merged app exactly as `main.rs` does, with a fake Stripe
/// gateway so the paid path can mint a session.
async fn build(
    pool: &SqlitePool,
    verifier: Option<Arc<dyn BotChallengeVerifier>>,
    cors_origins: Option<String>,
) -> Router {
    // An admin has to exist or the setup middleware redirects every
    // request to /setup — including this public page.
    let admin = make_member(pool).await;
    sqlx::query("UPDATE members SET is_admin = 1 WHERE id = ?")
        .bind(admin.to_string())
        .execute(pool)
        .await
        .unwrap();

    let payment_repo: Arc<dyn PaymentRepository> =
        Arc::new(SqlitePaymentRepository::new(pool.clone()));
    let member_repo: Arc<dyn MemberRepository> =
        Arc::new(SqliteMemberRepository::new(pool.clone()));
    let gw: Arc<dyn StripeGateway> = Arc::new(FakeStripeGateway::new());
    let client = Arc::new(StripeClient::with_gateway(gw, payment_repo, member_repo));
    let handle = Arc::new(StripeHandle::preloaded(Some(client), None));

    let state = build_app_state_full(pool.clone(), Some(handle), verifier, cors_origins).await;

    coterie::api::create_app(state.clone())
        .merge(coterie::web::create_web_routes(state.clone()))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            coterie::api::middleware::setup::require_setup,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state,
            coterie::api::middleware::security::csrf_protect_unless_exempt,
        ))
}

async fn make_event(
    pool: &SqlitePool,
    visibility: EventVisibility,
    member_price_cents: i64,
    guest_price_cents: i64,
    guest_registration_enabled: bool,
    max_attendees: Option<i32>,
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
        max_attendees,
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

async fn get(app: &Router, uri: &str) -> (StatusCode, String) {
    let resp = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).to_string())
}

/// POST the registration form the way Coterie's own page does
/// (urlencoded, no session, no CSRF token).
async fn post_form(app: &Router, event_id: Uuid, body: &str) -> (StatusCode, Option<String>) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/public/events/{}/register", event_id))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let location = resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    (status, location)
}

async fn guest_status(pool: &SqlitePool, event_id: Uuid, email: &str) -> Option<AttendanceStatus> {
    SqliteEventRepository::new(pool.clone())
        .attendance_status(
            event_id,
            &Attendee::Guest {
                name: "ignored".to_string(),
                email: email.to_string(),
            },
        )
        .await
        .unwrap()
}

async fn payment_count(pool: &SqlitePool) -> i64 {
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM payments")
        .fetch_one(pool)
        .await
        .unwrap();
    n
}

async fn seat_count(pool: &SqlitePool) -> i64 {
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM event_attendance")
        .fetch_one(pool)
        .await
        .unwrap();
    n
}

// ---------------------------------------------------------------------
// 7.1 — enumeration: private and absent must look the same
// ---------------------------------------------------------------------

#[tokio::test]
async fn members_only_and_nonexistent_events_produce_identical_404s() {
    let pool = fresh_pool().await;
    let app = build(&pool, None, None).await;
    // Guest registration is even ENABLED on the members-only event, to
    // prove visibility alone closes the public door.
    let private = make_event(&pool, EventVisibility::MembersOnly, 0, 3000, true, None).await;
    let missing = Uuid::new_v4();

    let (private_status, private_body) =
        get(&app, &format!("/events/{}/register", private.id)).await;
    let (missing_status, missing_body) = get(&app, &format!("/events/{}/register", missing)).await;

    assert_eq!(private_status, StatusCode::NOT_FOUND);
    assert_eq!(missing_status, StatusCode::NOT_FOUND);
    assert_eq!(
        private_body, missing_body,
        "a members-only id must be indistinguishable from a nonexistent one",
    );
    assert!(
        !private_body.contains("Lockpicking"),
        "the 404 discloses nothing about the event: {private_body}",
    );

    // The money endpoint applies the same rule.
    let (status, _) = post_form(&app, private.id, GUEST_FORM).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = post_form(&app, missing, GUEST_FORM).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(seat_count(&pool).await, 0);
    assert_eq!(payment_count(&pool).await, 0);
}

// ---------------------------------------------------------------------
// 7.2 — a zero price does not open a public door
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_public_event_without_guest_registration_is_not_registerable() {
    let pool = fresh_pool().await;
    let app = build(&pool, None, None).await;
    // The common case: a free public talk anyone walks into.
    let show_up = make_event(&pool, EventVisibility::Public, 0, 0, false, None).await;

    let (status, _) = get(&app, &format!("/events/{}/register", show_up.id)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = post_form(&app, show_up.id, GUEST_FORM).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(seat_count(&pool).await, 0);
}

// ---------------------------------------------------------------------
// 7.2b — free registerable event: page, seat, and feed
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_free_registerable_event_serves_the_page_and_confirms_a_seat() {
    let pool = fresh_pool().await;
    let app = build(&pool, None, None).await;
    let event = make_event(&pool, EventVisibility::Public, 0, 0, true, Some(20)).await;

    let (status, body) = get(&app, &format!("/events/{}/register", event.id)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Lockpicking 101"));
    assert!(
        body.contains("Free"),
        "a zero price renders as free: {body}"
    );
    assert!(body.contains("20 seats remaining"));
    assert!(
        body.contains(&format!("/public/events/{}/register", event.id)),
        "the page posts to the money endpoint",
    );
    // The roster is not public information.
    assert!(!body.to_lowercase().contains("roster"));

    let (status, location) = post_form(&app, event.id, GUEST_FORM).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(
        location
            .as_deref()
            .unwrap_or_default()
            .contains("registered=1"),
        "a free registration returns to the page, not to a payment provider: {location:?}",
    );
    assert_eq!(
        guest_status(&pool, event.id, GUEST_EMAIL).await,
        Some(AttendanceStatus::Registered),
    );
    assert_eq!(
        payment_count(&pool).await,
        0,
        "a free registration creates no payment row",
    );
}

// ---------------------------------------------------------------------
// 7.2b / 7.2c — the feed's one field decides the marketing site's UI
// ---------------------------------------------------------------------

#[tokio::test]
async fn public_events_feed_carries_registration_url_only_when_registerable() {
    let pool = fresh_pool().await;
    let app = build(&pool, None, None).await;

    let free = make_event(&pool, EventVisibility::Public, 0, 0, true, Some(20)).await;
    let paid = make_event(&pool, EventVisibility::Public, 1000, 3000, true, None).await;
    let show_up = make_event(&pool, EventVisibility::Public, 0, 0, false, None).await;
    // Guest registration enabled but members-only: still never advertised.
    let private = make_event(&pool, EventVisibility::MembersOnly, 0, 3000, true, None).await;

    let (status, body) = get(&app, "/public/events").await;
    assert_eq!(status, StatusCode::OK);
    let feed: serde_json::Value = serde_json::from_str(&body).unwrap();
    let entry = |id: Uuid| {
        feed.as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"].as_str() == Some(&id.to_string()))
            .cloned()
            .unwrap_or_else(|| panic!("event {id} is in the feed"))
    };

    let free_entry = entry(free.id);
    assert_eq!(
        free_entry["registration_url"].as_str(),
        Some(format!("http://127.0.0.1/events/{}/register", free.id).as_str()),
        "a zero price must not suppress the URL",
    );
    assert_eq!(free_entry["guest_price_cents"].as_i64(), Some(0));

    let paid_entry = entry(paid.id);
    assert_eq!(
        paid_entry["registration_url"].as_str(),
        Some(format!("http://127.0.0.1/events/{}/register", paid.id).as_str()),
    );
    assert_eq!(paid_entry["guest_price_cents"].as_i64(), Some(3000));

    // The common case: no registration affordance at all.
    let show_up_entry = entry(show_up.id);
    assert!(show_up_entry["registration_url"].is_null());
    assert!(show_up_entry["guest_price_cents"].is_null());

    let private_entry = entry(private.id);
    assert!(
        private_entry["registration_url"].is_null(),
        "a members-only event never advertises registration",
    );
    assert!(private_entry["guest_price_cents"].is_null());
    assert_eq!(private_entry["title"].as_str(), Some("Members-Only Event"));
}

// ---------------------------------------------------------------------
// 4.2 / 4.3 — the page's two other jobs
// ---------------------------------------------------------------------

#[tokio::test]
async fn the_page_offers_member_pricing_and_hides_the_form_when_full() {
    let pool = fresh_pool().await;
    let app = build(&pool, None, None).await;

    // Members attend free, guests pay — the case where an unknowing
    // member overpays the most.
    let event = make_event(&pool, EventVisibility::Public, 0, 3000, true, Some(1)).await;
    let (status, body) = get(&app, &format!("/events/{}/register", event.id)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("$30.00"));
    assert!(
        body.contains("Members pay free") && body.contains("/login"),
        "a member is told before paying, with a way to log in: {body}",
    );

    // Fill the only seat, then the page says so and offers no form.
    let (status, _) = post_form(&app, event.id, GUEST_FORM).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let (status, body) = get(&app, &format!("/events/{}/register", event.id)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("This event is full"));
    assert!(
        !body.contains("<form"),
        "a sold-out page renders no registration form: {body}",
    );
}

// ---------------------------------------------------------------------
// 5.3 / 5.5 — CSRF exemption reaches the parameterized path
// ---------------------------------------------------------------------

#[tokio::test]
async fn the_register_endpoint_is_csrf_exempt_with_no_session() {
    let pool = fresh_pool().await;
    let app = build(&pool, None, None).await;
    let event = make_event(&pool, EventVisibility::Public, 0, 0, true, None).await;

    // No session cookie, no token, no header. If the exempt list matched
    // paths by string equality, the `:id` entry would never match and
    // this would be a 403 the caller can do nothing about.
    let (status, _) = post_form(&app, event.id, GUEST_FORM).await;
    assert_ne!(
        status,
        StatusCode::FORBIDDEN,
        "the anonymous caller has no session to bind a token to",
    );
    assert_eq!(status, StatusCode::SEE_OTHER);

    // A neighbouring path is NOT exempted by the same entry.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/public/events/{}/register/extra", event.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// ---------------------------------------------------------------------
// 7.8 — the protections, in order, writing nothing when they fire
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_rate_limited_registration_never_reaches_the_provider_or_a_seat() {
    let pool = fresh_pool().await;
    // Counting verifier: the assertion is that it is NOT consulted by
    // the request the limiter rejects.
    let verifier = Arc::new(FakeVerifier::new(|_t| Ok(())));
    let app = build(&pool, Some(verifier.clone()), None).await;
    // Free + uncapped, so nothing but the limiter can reject.
    let event = make_event(&pool, EventVisibility::Public, 0, 0, true, None).await;

    // The money limiter's budget in tests is 10 per 60s.
    for i in 0..10 {
        let (status, _) = post_form(
            &app,
            event.id,
            &format!("name=Guest+{i}&email=g{i}%40example.com&captcha_token=tok"),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::SEE_OTHER,
            "request {i} should be served"
        );
    }
    assert_eq!(verifier.call_count(), 10);

    let (status, _) = post_form(
        &app,
        event.id,
        "name=Over&email=over%40example.com&captcha_token=tok",
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        verifier.call_count(),
        10,
        "the limiter runs BEFORE the provider, so a bursting IP can't burn the quota",
    );
    assert_eq!(
        guest_status(&pool, event.id, "over@example.com").await,
        None,
        "a rate-limited request claims no seat",
    );
    assert_eq!(seat_count(&pool).await, 10);
}

#[tokio::test]
async fn a_failed_bot_challenge_writes_nothing() {
    let pool = fresh_pool().await;
    let verifier = Arc::new(FakeVerifier::new(|_t| {
        Err(VerifyError::Invalid {
            provider_codes: vec!["bad-token".to_string()],
        })
    }));
    let app = build(&pool, Some(verifier), None).await;
    // Paid, so a bypass would also mint a Checkout session.
    let event = make_event(&pool, EventVisibility::Public, 0, 3000, true, None).await;

    let (status, _) = post_form(&app, event.id, &format!("{GUEST_FORM}&captcha_token=tok")).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "fail closed");
    assert_eq!(guest_status(&pool, event.id, GUEST_EMAIL).await, None);
    assert_eq!(seat_count(&pool).await, 0, "no seat claimed");
    assert_eq!(payment_count(&pool).await, 0, "no payment row created");
}

// ---------------------------------------------------------------------
// 5.4 — the CORS allowlist covers the endpoint for the marketing origin
// ---------------------------------------------------------------------

#[tokio::test]
async fn the_marketing_origin_is_allowed_to_call_the_register_endpoint() {
    let pool = fresh_pool().await;
    let origin = "https://neontemple.example";
    let app = build(&pool, None, Some(origin.to_string())).await;
    let event = make_event(&pool, EventVisibility::Public, 0, 0, true, None).await;

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/public/events/{}/register", event.id))
                .header("origin", origin)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "name": "Ada", "email": GUEST_EMAIL }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some(origin),
        "the same allowlist that covers /public/signup covers this endpoint",
    );
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "and the JSON caller is served"
    );

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"].as_str(), Some("registered"));
    assert!(
        json.get("checkout_url").is_none(),
        "a free registration owes no payment: {json}",
    );
}

// ---------------------------------------------------------------------
// The paid path through the endpoint: a held seat and a Stripe redirect
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_paid_guest_registration_holds_the_seat_and_redirects_to_checkout() {
    let pool = fresh_pool().await;
    let app = build(&pool, None, None).await;
    let event = make_event(&pool, EventVisibility::Public, 0, 3000, true, Some(2)).await;

    let (status, location) = post_form(&app, event.id, GUEST_FORM).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(
        location
            .as_deref()
            .unwrap_or_default()
            .contains("checkout.example.test")
            || location.as_deref().unwrap_or_default().contains("stripe"),
        "the browser is sent to the payment provider: {location:?}",
    );
    assert_eq!(
        guest_status(&pool, event.id, GUEST_EMAIL).await,
        Some(AttendanceStatus::PendingPayment),
        "the seat is held, not confirmed — only the webhook confirms it",
    );
    assert_eq!(payment_count(&pool).await, 1, "one Pending placeholder");
}
