//! End-to-end router tests for the saved-card surface after the
//! a03-narrow-saved-card-json-surface change.
//!
//! After the change, the saved-card surface looks like:
//!
//!   JSON  POST   /api/payments/cards/setup-intent       (kept; Stripe.js)
//!   JSON  POST   /api/payments/cards                    (kept; Stripe.js)
//!   HTML  GET    /portal/api/payments/cards             (kept; HTMX)
//!   HTML  DELETE /portal/api/payments/cards/:id         (kept; HTMX)
//!   HTML  PUT    /portal/api/payments/cards/:id/default (kept; HTMX)
//!
//! The previously-parallel JSON list / delete / set-default endpoints
//! were deleted as vestigial. These tests prove the kept endpoints
//! still work end-to-end through the merged router AND that the
//! deleted ones now 404 — the regression net that says "yes, the
//! route really got unregistered."
//!
//! Run with: cargo test --features test-utils --test saved_card_routes_test

use std::sync::Arc;

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
    Router,
};
use chrono::Utc;
use coterie::{
    auth::{AuthService, CsrfService, PendingLoginService, SecretCrypto, TotpService},
    config::Settings,
    domain::{CreateMemberRequest, SavedCard},
    email::LogSender,
    integrations::IntegrationManager,
    payments::{
        fake_gateway::FakeStripeGateway, gateway::StripeGateway, StripeClient,
    },
    repository::{
        AnnouncementRepository, EventRepository, MemberRepository, PaymentRepository,
        SavedCardRepository, SqliteAnnouncementRepository, SqliteEventRepository,
        SqliteMemberRepository, SqlitePaymentRepository, SqliteSavedCardRepository,
    },
    service::{settings_service::SettingsService, ServiceContext},
};
use sqlx::{Executor, SqlitePool};
use tower::ServiceExt;
use uuid::Uuid;

// ---------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------

/// Wraps everything a test needs to drive the merged router: the router
/// itself, the pool (for direct DB seeding / assertions), the fake
/// gateway (to inspect / prime Stripe calls), the saved-card repo
/// (for seeding rows), and an active test member's id + a valid
/// session cookie + a valid CSRF token bound to that session.
struct Harness {
    app: Router,
    pool: SqlitePool,
    fake: Arc<FakeStripeGateway>,
    saved_card_repo: Arc<dyn SavedCardRepository>,
    member_id: Uuid,
    session_cookie: String,
    csrf_token: String,
}

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
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("migrate");
    pool
}

/// Build the merged app exactly the way `main.rs` does — both the
/// /api and /portal surfaces under the same top-level CSRF + setup
/// layers — and pre-create an Active member with a valid session.
async fn build_harness() -> Harness {
    let pool = fresh_pool().await;

    let settings = Settings {
        server: coterie::config::ServerConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            base_url: "http://127.0.0.1".to_string(),
            data_dir: "./data".to_string(),
            uploads_dir: None,
            secure_cookies: Some(false),
            cors_origins: None,
            trust_forwarded_for: Some(false),
        },
        database: coterie::config::DatabaseConfig {
            url: "sqlite::memory:".to_string(),
            max_connections: 1,
        },
        auth: coterie::config::AuthConfig {
            session_secret: "test-session-secret-please-ignore".to_string(),
            session_duration_hours: 24,
            totp_issuer: "Coterie Test".to_string(),
        },
        stripe: Default::default(),
        integrations: Default::default(),
        seed: Default::default(),
        bot_challenge: Default::default(),
    };
    let settings = Arc::new(settings);

    let member_repo: Arc<dyn MemberRepository> =
        Arc::new(SqliteMemberRepository::new(pool.clone()));
    let event_repo: Arc<dyn EventRepository> =
        Arc::new(SqliteEventRepository::new(pool.clone()));
    let announcement_repo: Arc<dyn AnnouncementRepository> =
        Arc::new(SqliteAnnouncementRepository::new(pool.clone()));
    let payment_repo: Arc<dyn PaymentRepository> =
        Arc::new(SqlitePaymentRepository::new(pool.clone()));
    let saved_card_repo: Arc<dyn SavedCardRepository> =
        Arc::new(SqliteSavedCardRepository::new(pool.clone()));

    let crypto = Arc::new(SecretCrypto::new("test-secret-please-ignore"));
    let auth_service = Arc::new(AuthService::new(
        pool.clone(),
        settings.auth.session_secret.clone(),
    ));
    let csrf_service = Arc::new(CsrfService::new(&settings.auth.session_secret));
    let totp_service = Arc::new(TotpService::new(
        pool.clone(),
        crypto.clone(),
        "Coterie".to_string(),
    ));
    let pending_login_service = Arc::new(PendingLoginService::new(pool.clone()));
    let settings_service = Arc::new(SettingsService::new(pool.clone(), crypto));

    let email_sender = Arc::new(LogSender::new(
        "test@example.com".to_string(),
        "Test".to_string(),
    ));
    let integration_manager = Arc::new(IntegrationManager::new());

    let service_context = Arc::new(ServiceContext::new(
        member_repo.clone(),
        event_repo,
        announcement_repo,
        payment_repo.clone(),
        integration_manager,
        auth_service.clone(),
        email_sender,
        settings_service,
        csrf_service.clone(),
        totp_service,
        pending_login_service,
        settings.server.base_url.clone(),
        pool.clone(),
    ));

    // Fake Stripe gateway: wire BOTH the StripeClient (outbound) and
    // the AppState slot, so the SetupIntent / save-card handlers reach
    // a fake rather than panicking on a missing client.
    let fake = Arc::new(FakeStripeGateway::new());
    let gw: Arc<dyn StripeGateway> = fake.clone();
    let stripe_client = Arc::new(StripeClient::with_gateway(
        gw,
        payment_repo,
        member_repo.clone(),
    ));

    let billing_service = Arc::new(service_context.billing_service(
        Some(stripe_client.clone()),
        settings.server.base_url.clone(),
    ));

    let app_state = coterie::api::state::AppState::new(
        service_context.clone(),
        Some(stripe_client),
        None, // webhook_dispatcher not needed for these tests
        billing_service,
        settings,
        Arc::new(coterie::api::middleware::bot_challenge::DisabledVerifier),
    );

    // Create an Active test member. The default status from
    // `MemberRepository::create` is `Pending`, which `require_auth`
    // rejects — flip to Active so the JSON endpoints accept the
    // session. Also mark admin = true to satisfy `require_setup`
    // (which redirects when no admin exists in the DB).
    let created = member_repo
        .create(CreateMemberRequest {
            email: "test-member@example.com".to_string(),
            username: "test_member".to_string(),
            full_name: "Test Member".to_string(),
            password: "p4ssword_long_enough".to_string(),
            membership_type_id: None,
        })
        .await
        .expect("create test member");
    sqlx::query(
        "UPDATE members SET status = 'Active', is_admin = 1, \
         stripe_customer_id = 'cus_test_member' WHERE id = ?",
    )
    .bind(created.id.to_string())
    .execute(&pool)
    .await
    .expect("flip status + admin + customer id");

    // Real session + real CSRF token, so we go through the same code
    // paths as a real browser request (rather than fabricating cookie /
    // token formats that might drift from production).
    let (session, token) = auth_service
        .create_session(created.id, 24)
        .await
        .expect("create session");
    let csrf_token = csrf_service
        .generate_token(&session.id)
        .await
        .expect("generate csrf token");
    let session_cookie = format!("session={}", token);

    let api_app = coterie::api::create_app(app_state.clone());
    let web_app = coterie::web::create_web_routes(app_state.clone());

    let app = api_app
        .merge(web_app)
        .layer(axum::middleware::from_fn_with_state(
            app_state.clone(),
            coterie::api::middleware::setup::require_setup,
        ))
        .layer(axum::middleware::from_fn_with_state(
            app_state,
            coterie::api::middleware::security::csrf_protect_unless_exempt,
        ));

    Harness {
        app,
        pool,
        fake,
        saved_card_repo,
        member_id: created.id,
        session_cookie,
        csrf_token,
    }
}

/// Authenticated request builder — stamps the session cookie and CSRF
/// token so we get past the CSRF layer and the auth gate.
fn auth_request(
    h: &Harness,
    method: &str,
    uri: &str,
    body: Body,
    content_type: Option<&str>,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::COOKIE, &h.session_cookie)
        .header("X-CSRF-Token", &h.csrf_token);
    if let Some(ct) = content_type {
        builder = builder.header(header::CONTENT_TYPE, ct);
    }
    builder.body(body).unwrap()
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[tokio::test]
async fn deleted_json_endpoints_have_no_handler() {
    // The three JSON endpoints we removed. After the change, each
    // request must not reach a working handler:
    //
    //   GET /api/payments/cards
    //     → 405 (the POST route is still registered at this path, so
    //       axum returns Method Not Allowed rather than Not Found —
    //       semantically still "no handler for this method+path").
    //
    //   DELETE /api/payments/cards/:id
    //   PUT    /api/payments/cards/:id/default
    //     → 404 (no route matches the path at all).
    //
    // Send a valid session + CSRF so we get past the top-level CSRF
    // layer for DELETE / PUT. Without those the layer would 403
    // *before* the router decides, masking the regression we care
    // about (route-not-registered).
    let h = build_harness().await;
    let card_id = Uuid::new_v4();

    let get_req = Request::builder()
        .method("GET")
        .uri("/api/payments/cards")
        .header(header::COOKIE, &h.session_cookie)
        .body(Body::empty())
        .unwrap();
    let get_resp = h.app.clone().oneshot(get_req).await.unwrap();
    assert_eq!(
        get_resp.status(),
        StatusCode::METHOD_NOT_ALLOWED,
        "GET /api/payments/cards must not reach a handler after the route was unregistered \
         (axum returns 405 because the POST route still occupies this path)"
    );

    let delete_req = auth_request(
        &h,
        "DELETE",
        &format!("/api/payments/cards/{}", card_id),
        Body::empty(),
        None,
    );
    let delete_resp = h.app.clone().oneshot(delete_req).await.unwrap();
    assert_eq!(
        delete_resp.status(),
        StatusCode::NOT_FOUND,
        "DELETE /api/payments/cards/:id must 404 after the route was unregistered"
    );

    let put_req = auth_request(
        &h,
        "PUT",
        &format!("/api/payments/cards/{}/default", card_id),
        Body::empty(),
        None,
    );
    let put_resp = h.app.clone().oneshot(put_req).await.unwrap();
    assert_eq!(
        put_resp.status(),
        StatusCode::NOT_FOUND,
        "PUT /api/payments/cards/:id/default must 404 after the route was unregistered"
    );
}

#[tokio::test]
async fn setup_intent_flow_still_works() {
    // POST /api/payments/cards/setup-intent stays; this is one of the
    // two endpoints Stripe.js calls directly. The kept handler must
    // route through the gateway and surface the client_secret.
    let h = build_harness().await;

    let req = auth_request(
        &h,
        "POST",
        "/api/payments/cards/setup-intent",
        Body::empty(),
        Some("application/json"),
    );
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "POST /api/payments/cards/setup-intent must reach the handler"
    );

    let body = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).expect("body is JSON");
    assert!(
        json.get("client_secret")
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false),
        "response must carry a non-empty client_secret; got {}",
        json,
    );

    // The fake gateway should have recorded a CreateSetupIntent call,
    // confirming the request actually flowed through StripeClient and
    // wasn't short-circuited.
    let setup_intent_calls = h.fake.count_where(|c| matches!(
        c,
        coterie::payments::fake_gateway::FakeCall::CreateSetupIntent(_)
    ));
    assert_eq!(
        setup_intent_calls, 1,
        "exactly one CreateSetupIntent call expected on the fake gateway"
    );
}

#[tokio::test]
async fn save_card_flow_still_works() {
    // POST /api/payments/cards stays; called by Stripe.js after it
    // confirms the SetupIntent client-side. The default
    // FakeStripeGateway `retrieve_payment_method` response returns a
    // PM with `customer_id: None`, which the handler treats as a
    // fresh SetupIntent PM (its cross-member-stapling guard allows
    // this case explicitly). That's the happy path we're proving here.
    let h = build_harness().await;

    let body = serde_json::json!({
        "stripe_payment_method_id": "pm_card_visa",
        "set_as_default": true,
    })
    .to_string();

    let req = auth_request(
        &h,
        "POST",
        "/api/payments/cards",
        Body::from(body),
        Some("application/json"),
    );
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::CREATED,
        "POST /api/payments/cards must return 201 CREATED on the happy path"
    );

    // The handler must have persisted a saved_cards row for this
    // member. Find-by-member is the same path the HTML list endpoint
    // uses, so this also indirectly proves the new row would show up
    // in the fragment.
    let cards = h
        .saved_card_repo
        .find_by_member(h.member_id)
        .await
        .expect("find_by_member ok");
    assert_eq!(cards.len(), 1, "exactly one saved card after the save");
    assert_eq!(cards[0].stripe_payment_method_id, "pm_card_visa");
}

#[tokio::test]
async fn html_list_endpoint_returns_fragment() {
    // GET /portal/api/payments/cards stays; this is the HTMX swap
    // target on the payment-methods page. Must return HTML, not JSON,
    // and the body must look like the `_saved_card_list.html`
    // template's rendered output (we check for the "No saved
    // payment methods." marker — easy and stable when no cards are
    // seeded).
    let h = build_harness().await;

    let req = Request::builder()
        .method("GET")
        .uri("/portal/api/payments/cards")
        .header(header::COOKIE, &h.session_cookie)
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "GET /portal/api/payments/cards must reach the handler"
    );

    let ct = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(
        ct.starts_with("text/html"),
        "content-type must be text/html; got {}",
        ct,
    );

    let body = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let body_str = String::from_utf8_lossy(&body);
    assert!(
        body_str.contains("No saved payment methods"),
        "empty-list marker should appear in the rendered fragment; got body:\n{}",
        body_str,
    );
}

#[tokio::test]
async fn html_delete_endpoint_works() {
    // DELETE /portal/api/payments/cards/:id stays; HTMX uses this to
    // remove a card. Seed a row directly, fire the request, assert
    // the row is gone.
    let h = build_harness().await;
    let card = seed_card(&h, "pm_one", true).await;

    let req = auth_request(
        &h,
        "DELETE",
        &format!("/portal/api/payments/cards/{}", card.id),
        Body::empty(),
        None,
    );
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "DELETE /portal/api/payments/cards/:id must return 200 on success"
    );

    let remaining = h
        .saved_card_repo
        .find_by_member(h.member_id)
        .await
        .expect("find_by_member ok");
    assert!(
        remaining.is_empty(),
        "card row should be gone after delete; got {:?}",
        remaining,
    );
}

#[tokio::test]
async fn html_set_default_endpoint_works() {
    // PUT /portal/api/payments/cards/:id/default stays; HTMX uses
    // this to swap which card is marked default. Seed two cards —
    // first one default, second one not — then mark the second one
    // default and assert the swap.
    let h = build_harness().await;
    let first = seed_card(&h, "pm_first", true).await;
    let second = seed_card(&h, "pm_second", false).await;

    let req = auth_request(
        &h,
        "PUT",
        &format!("/portal/api/payments/cards/{}/default", second.id),
        Body::empty(),
        None,
    );
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "PUT /portal/api/payments/cards/:id/default must return 200 on success"
    );

    let after = h
        .saved_card_repo
        .find_by_member(h.member_id)
        .await
        .expect("find_by_member ok");
    let first_after = after.iter().find(|c| c.id == first.id).unwrap();
    let second_after = after.iter().find(|c| c.id == second.id).unwrap();
    assert!(
        !first_after.is_default,
        "previously-default card must be demoted"
    );
    assert!(
        second_after.is_default,
        "newly-promoted card must be marked default"
    );
}

// ---------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------

async fn seed_card(h: &Harness, pm_id: &str, is_default: bool) -> SavedCard {
    let now = Utc::now();
    h.saved_card_repo
        .create(SavedCard {
            id: Uuid::new_v4(),
            member_id: h.member_id,
            stripe_payment_method_id: pm_id.to_string(),
            card_last_four: "4242".to_string(),
            card_brand: "visa".to_string(),
            exp_month: 12,
            exp_year: 2030,
            is_default,
            created_at: now,
            updated_at: now,
        })
        .await
        .expect("seed saved_card row")
}

// `pool` is held on Harness so direct DB inspection from tests stays
// possible. Silence the dead-code lint if a test removes its only use.
#[allow(dead_code)]
fn _pool_used_by_harness(h: &Harness) -> &SqlitePool {
    &h.pool
}
