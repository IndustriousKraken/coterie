//! Tests for the `require_setup` middleware's process-cached
//! admin-existence flag (`AppState::admin_exists_observed`).
//!
//! Cache contract under test:
//!
//! * First positive lookup arms the flag for the rest of the process.
//! * Once armed, the middleware skips the DB query entirely — even if
//!   the DB later disagrees (e.g. operator manually truncates the
//!   `members` table).
//! * Negative lookups (no admin yet) leave the flag `false` and
//!   redirect to `/setup`.
//! * The setup-wizard handler (`POST /setup`) proactively arms the
//!   flag immediately after creating the first admin, so the very
//!   next request bypasses the redundant DB query.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use coterie::{
    api::{
        middleware::bot_challenge::DisabledVerifier,
        state::AppState,
    },
    auth::{AuthService, CsrfService, PendingLoginService, SecretCrypto, TotpService},
    config::Settings,
    domain::{CreateMemberRequest, MemberStatus, UpdateMemberRequest},
    email::LogSender,
    integrations::IntegrationManager,
    repository::{
        AnnouncementRepository, EventRepository, MemberRepository, PaymentRepository,
        SqliteAnnouncementRepository, SqliteEventRepository, SqliteMemberRepository,
        SqlitePaymentRepository,
    },
    service::{settings_service::SettingsService, ServiceContext},
};
use sqlx::{Executor, SqlitePool};
use tower::ServiceExt;

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

async fn build_app_state(pool: SqlitePool) -> AppState {
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
        member_repo,
        event_repo,
        announcement_repo,
        payment_repo,
        integration_manager,
        auth_service,
        email_sender,
        settings_service,
        csrf_service,
        totp_service,
        pending_login_service,
        settings.server.base_url.clone(),
        pool.clone(),
    ));

    let billing_service = Arc::new(service_context.billing_service(
        None,
        settings.server.base_url.clone(),
    ));

    AppState::new(
        service_context,
        None,
        None,
        billing_service,
        settings,
        Arc::new(DisabledVerifier),
    )
}

/// Seed a single admin row so `check_admin_exists` returns `true`.
async fn seed_admin(state: &AppState) {
    let create = CreateMemberRequest {
        email: "seed-admin@example.com".to_string(),
        username: "seedadmin".to_string(),
        full_name: "Seed Admin".to_string(),
        password: "SeedPassword1".to_string(),
        membership_type_id: None,
    };
    let member = state
        .service_context
        .member_repo
        .create(create)
        .await
        .expect("create seed admin");
    let update = UpdateMemberRequest {
        status: Some(MemberStatus::Active),
        bypass_dues: Some(true),
        ..Default::default()
    };
    state
        .service_context
        .member_repo
        .update(member.id, update)
        .await
        .expect("activate seed admin");
    state
        .service_context
        .member_repo
        .set_admin(member.id, true)
        .await
        .expect("promote seed admin");
}

/// Build a minimal router that exercises `require_setup` against a
/// dummy non-static path. The handler is an unconditional 200 so the
/// only way a response can become a redirect is via the middleware.
fn router_with_setup_layer(state: AppState) -> Router {
    use axum::{routing::get, http::StatusCode as Status};
    Router::new()
        .route("/dummy", get(|| async { Status::OK }))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            coterie::api::middleware::setup::require_setup,
        ))
        .with_state(state)
}

/// Build the merged router the way `main.rs` does, but WITHOUT the
/// CSRF layer. The integration tests in this file exercise the
/// setup-redirect path end-to-end, not CSRF, so omitting that layer
/// lets us drive `POST /setup` directly with a JSON body.
fn router_full(state: AppState) -> Router {
    let api_app = coterie::api::create_app(state.clone());
    let web_app = coterie::web::create_web_routes(state.clone());
    api_app
        .merge(web_app)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            coterie::api::middleware::setup::require_setup,
        ))
}

// ---------------------------------------------------------------------
// Section 4: unit tests for the middleware cache.
// ---------------------------------------------------------------------

/// Positive case: once the middleware sees an admin, it caches and
/// continues forwarding even after the DB is truncated to zero admins.
#[tokio::test]
async fn middleware_caches_first_positive_lookup() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool.clone()).await;
    seed_admin(&state).await;

    let app = router_with_setup_layer(state.clone());

    // 1st request: middleware queries the DB, sees an admin, forwards.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/dummy")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "first request should forward through (admin exists in DB)"
    );
    assert!(
        state.admin_exists_observed.load(Ordering::Relaxed),
        "cache should be armed after the first positive lookup"
    );

    // Truncate the members table — no admins exist in the DB anymore.
    sqlx::query("DELETE FROM members")
        .execute(&pool)
        .await
        .expect("truncate members");

    // 2nd request: the DB now says "no admin", but the cache still
    // says "yes". The middleware must forward without re-querying. If
    // the cache weren't real, this request would 303 to /setup.
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/dummy")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "second request should still forward (cache trumps stale DB)"
    );
}

/// Negative case: no admin in DB → middleware redirects, cache stays
/// `false`.
#[tokio::test]
async fn middleware_redirects_when_no_admin_and_cache_stays_false() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool).await;
    let app = router_with_setup_layer(state.clone());

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/dummy")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::SEE_OTHER,
        "no admin should redirect to /setup (303 See Other)"
    );
    let location = resp
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(location, "/setup", "redirect target should be /setup");
    assert!(
        !state.admin_exists_observed.load(Ordering::Relaxed),
        "cache must remain false after a negative lookup"
    );
}

// ---------------------------------------------------------------------
// Section 5: integration test of the wizard → cache transition.
// ---------------------------------------------------------------------

/// End-to-end: a fresh instance redirects, completing the wizard arms
/// the cache, and the subsequent request forwards without redirect.
#[tokio::test]
async fn wizard_post_arms_cache_for_next_request() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool).await;
    let app = router_full(state.clone());

    // 5.2: anonymous request to a non-static, non-setup path → 303 to
    // /setup, cache still false.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/portal")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::SEE_OTHER,
        "pre-setup request to /portal should redirect to /setup"
    );
    assert_eq!(
        resp.headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok()),
        Some("/setup")
    );
    assert!(
        !state.admin_exists_observed.load(Ordering::Relaxed),
        "cache should still be false before the wizard runs"
    );

    // 5.3: drive POST /setup with valid form data → 200, cache armed.
    let body = serde_json::json!({
        "org_name": "Test Org",
        "email": "admin@example.com",
        "username": "admin",
        "full_name": "Admin User",
        "password": "WizardPass1",
        "password_confirm": "WizardPass1",
    });
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/setup")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "wizard POST should succeed with valid form data"
    );
    assert!(
        state.admin_exists_observed.load(Ordering::Relaxed),
        "wizard handler must proactively arm the cache after creating \
         the first admin"
    );

    // 5.4: follow-up request to the same non-static path forwards
    // through (no redirect). The cache being armed means
    // `check_admin_exists` is never reached on this request — see the
    // Section 4 unit tests, which already prove the no-query-when-
    // cached path by truncating the members table between requests.
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/portal")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(
        resp.status(),
        StatusCode::SEE_OTHER.as_u16(),
        // belt: also covered by the location-header check below
        "post-setup request should not be redirected to /setup; got {}",
        resp.status()
    );
    let location = resp
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_ne!(
        location, "/setup",
        "post-setup request must not Location: /setup; got {:?}",
        location
    );
}
