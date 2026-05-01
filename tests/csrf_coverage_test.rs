//! Regression test for F9: the top-level CSRF middleware must cover
//! every state-changing route, including the portal routes that get
//! added via `Router::merge`.
//!
//! The bug this guards against: in axum 0.7, layers applied to a
//! router *before* a `merge` call do not propagate to the merged
//! routes. CSRF was originally layered inside `api::create_app` and
//! the portal router was merged in afterwards in `main.rs`, leaving
//! every `/portal/*` POST/PUT/DELETE/PATCH unprotected.
//!
//! The fix moves the CSRF layer to wrap the merged app. This test
//! sends a POST to a state-changing portal admin route with no CSRF
//! token (and no session cookie) and asserts the middleware rejects
//! it with 403 Forbidden — proving the layer reaches `/portal/*`.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use coterie::{
    auth::{AuthService, CsrfService, PendingLoginService, SecretCrypto, TotpService},
    config::Settings,
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

/// Build the full merged app the way `main.rs` does. The whole point
/// of F9 is that a unit test of the middleware in isolation would pass
/// even with the original bug in place — the regression only shows up
/// at the routing layer where `.merge()` strips the layer.
async fn build_app() -> Router {
    let pool = fresh_pool().await;

    // --- Settings (minimal hand-built; the on-disk Settings::new()
    // expects .env / config files we don't want test deps on).
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
    };
    let settings = Arc::new(settings);

    // --- Repos
    let member_repo: Arc<dyn MemberRepository> =
        Arc::new(SqliteMemberRepository::new(pool.clone()));
    let event_repo: Arc<dyn EventRepository> =
        Arc::new(SqliteEventRepository::new(pool.clone()));
    let announcement_repo: Arc<dyn AnnouncementRepository> =
        Arc::new(SqliteAnnouncementRepository::new(pool.clone()));
    let payment_repo: Arc<dyn PaymentRepository> =
        Arc::new(SqlitePaymentRepository::new(pool.clone()));

    // --- Auth-related services
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
        pool.clone(),
    ));

    let billing_service = Arc::new(service_context.billing_service(
        None,
        settings.server.base_url.clone(),
    ));

    let api_app = coterie::api::create_app(
        service_context.clone(),
        None,
        None,
        billing_service.clone(),
        settings.clone(),
    );

    let web_app_state = coterie::api::state::AppState::new(
        service_context,
        None,
        None,
        billing_service,
        settings,
    );
    let web_app = coterie::web::create_web_routes(web_app_state.clone());

    // Mirror main.rs exactly: merge, then layer setup-check, then CSRF
    // (outermost). If F9 ever regresses (CSRF layered before merge),
    // the assertion in `portal_admin_post_without_csrf_returns_403`
    // will fail.
    api_app
        .merge(web_app)
        .layer(axum::middleware::from_fn_with_state(
            web_app_state.clone(),
            coterie::api::middleware::setup::require_setup,
        ))
        .layer(axum::middleware::from_fn_with_state(
            web_app_state,
            coterie::api::middleware::security::csrf_protect_unless_exempt,
        ))
}

#[tokio::test]
async fn portal_admin_post_without_csrf_returns_403() {
    let app = build_app().await;

    // POST a state-changing portal admin route. No session cookie,
    // no CSRF token of any form. Middleware must reject before any
    // handler / auth gate runs — the gate decision is "no session →
    // Forbidden" inside csrf_protect_unless_exempt itself.
    let req = Request::builder()
        .method("POST")
        .uri("/portal/admin/members/00000000-0000-0000-0000-000000000000/update")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();

    // The CSRF layer rejects with 403. If F9 regresses (layer doesn't
    // cover /portal/*), the request would fall through to the admin
    // gate, which redirects to /login (303) or 404s the path.
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "POST /portal/admin/members/.../update should be rejected by \
         the top-level CSRF layer with 403; got {}",
        resp.status()
    );
}

#[tokio::test]
async fn portal_admin_get_without_csrf_passes_through() {
    // Sanity: GETs are not state-changing and CSRF doesn't apply.
    // This proves the 403 above is from CSRF, not a blanket reject.
    let app = build_app().await;

    let req = Request::builder()
        .method("GET")
        .uri("/portal/admin/members")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();

    // Without a session, the admin gate redirects to /login. The
    // important bit: it's NOT a 403 from CSRF (which would mean CSRF
    // was incorrectly applying to GETs).
    assert_ne!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "GET /portal/admin/members should not be CSRF-rejected; got {}",
        resp.status()
    );
}

#[tokio::test]
async fn api_payments_post_without_csrf_returns_403() {
    // Confirms the layer also still covers the API surface (the
    // routes the original layer-inside-create_app did cover). This
    // catches a future regression where someone moves the layer
    // *too far* and accidentally drops API coverage.
    let app = build_app().await;

    let req = Request::builder()
        .method("POST")
        .uri("/api/payments/cards")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "POST /api/payments/cards should be rejected by CSRF; got {}",
        resp.status()
    );
}
