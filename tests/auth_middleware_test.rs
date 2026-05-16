//! Matrix tests for the four auth-middleware wrappers
//! (`require_auth`, `require_auth_redirect`, `require_restorable`,
//! `require_admin_redirect`).
//!
//! Each wrapper is exercised against the same set of caller shapes —
//! anonymous, Pending, Suspended, Expired, Active-non-admin,
//! Active-admin, Active-admin-without-TOTP-while-setting-on — and
//! asserts the wire-visible outcome (status code + Location header,
//! or 200 OK forwarded).
//!
//! These tests are the load-bearing safety net for the shared
//! `authenticate(...)` core: if any wrapper's reject behavior drifts,
//! the corresponding row of the matrix fails loudly.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    routing::get,
    Router,
};
use coterie::{
    api::{
        middleware::{
            auth::{
                require_admin_redirect, require_auth, require_auth_redirect,
                require_restorable,
            },
            bot_challenge::DisabledVerifier,
        },
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

    let billing_service = Arc::new(
        service_context.billing_service(None, settings.server.base_url.clone()),
    );

    AppState::new(
        service_context,
        None,
        None,
        billing_service,
        settings,
        Arc::new(DisabledVerifier),
    )
}

/// Create a member with the requested status / admin flag and mint a
/// fresh session for them. Returns the bare session token (cookie
/// value) so the caller can plant it on the test request.
async fn make_member_with_session(
    state: &AppState,
    status: MemberStatus,
    is_admin: bool,
) -> (Uuid, String) {
    let suffix = Uuid::new_v4();
    let member = state
        .service_context
        .member_repo
        .create(CreateMemberRequest {
            email: format!("u-{}@example.com", suffix),
            username: format!("user_{}", suffix.simple()),
            full_name: "Test User".to_string(),
            password: "p4ssword_long_enough".to_string(),
            membership_type_id: None,
        })
        .await
        .expect("create member");

    state
        .service_context
        .member_repo
        .update(
            member.id,
            UpdateMemberRequest {
                status: Some(status),
                ..Default::default()
            },
        )
        .await
        .expect("update status");

    if is_admin {
        state
            .service_context
            .member_repo
            .set_admin(member.id, true)
            .await
            .expect("set admin");
    }

    let (_, token) = state
        .service_context
        .auth_service
        .create_session(member.id, 24)
        .await
        .expect("create session");

    (member.id, token)
}

/// Stamp `auth.require_totp_for_admins` to `true` directly via SQL —
/// the public `update_setting` API demands an updater UUID and audit
/// trail, neither of which adds value here.
async fn set_admin_totp_required(pool: &SqlitePool, enabled: bool) {
    sqlx::query("UPDATE app_settings SET value = ? WHERE key = 'auth.require_totp_for_admins'")
        .bind(if enabled { "true" } else { "false" })
        .execute(pool)
        .await
        .expect("flip setting");
}

/// Stamp `totp_enabled_at` for the member so `is_enabled` returns true
/// without dragging in the full enrollment ceremony.
async fn force_totp_enabled(pool: &SqlitePool, member_id: Uuid) {
    sqlx::query("UPDATE members SET totp_enabled_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(member_id.to_string())
        .execute(pool)
        .await
        .expect("stamp totp_enabled_at");
}

/// Tiny "always 200" handler so the only way to see anything other
/// than 200 is for the middleware to short-circuit.
fn ok_router(state: AppState, mw: MiddlewareKind) -> Router {
    let base = || Router::new().route("/probe", get(|| async { StatusCode::OK }));
    match mw {
        MiddlewareKind::RequireAuth => base()
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                require_auth,
            ))
            .with_state(state),
        MiddlewareKind::RequireAuthRedirect => base()
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                require_auth_redirect,
            ))
            .with_state(state),
        MiddlewareKind::RequireRestorable => base()
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                require_restorable,
            ))
            .with_state(state),
        MiddlewareKind::RequireAdminRedirect => base()
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                require_admin_redirect,
            ))
            .with_state(state),
    }
}

#[derive(Copy, Clone)]
enum MiddlewareKind {
    RequireAuth,
    RequireAuthRedirect,
    RequireRestorable,
    RequireAdminRedirect,
}

fn req_with_cookie(token: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method("GET").uri("/probe");
    if let Some(t) = token {
        builder = builder.header(header::COOKIE, format!("session={}", t));
    }
    builder.body(Body::empty()).unwrap()
}

#[derive(Debug, PartialEq, Eq)]
enum Expected {
    Forwarded,
    Status(StatusCode),
    Redirect(&'static str),
}

async fn run_one(
    state: &AppState,
    mw: MiddlewareKind,
    token: Option<&str>,
) -> Expected {
    let app = ok_router(state.clone(), mw);
    let resp = app.oneshot(req_with_cookie(token)).await.unwrap();
    match resp.status() {
        StatusCode::OK => Expected::Forwarded,
        StatusCode::SEE_OTHER | StatusCode::TEMPORARY_REDIRECT | StatusCode::FOUND => {
            let loc = resp
                .headers()
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            // Leak to 'static so it slots into the enum cleanly for
            // assert_eq! display; the value lives only for the test.
            Expected::Redirect(Box::leak(loc.into_boxed_str()))
        }
        other => Expected::Status(other),
    }
}

#[tokio::test]
async fn access_policy_matrix() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool.clone()).await;

    // Seed one member per status; admin permutations on top of Active.
    let (_, tok_pending) = make_member_with_session(&state, MemberStatus::Pending, false).await;
    let (_, tok_suspended) =
        make_member_with_session(&state, MemberStatus::Suspended, false).await;
    let (_, tok_expired) = make_member_with_session(&state, MemberStatus::Expired, false).await;
    let (_, tok_active) = make_member_with_session(&state, MemberStatus::Active, false).await;
    let (admin_id, tok_admin) =
        make_member_with_session(&state, MemberStatus::Active, true).await;

    // ---- require_auth: JSON-style 401/403, never redirects ----
    let mw = MiddlewareKind::RequireAuth;
    assert_eq!(
        run_one(&state, mw, None).await,
        Expected::Status(StatusCode::UNAUTHORIZED),
        "anonymous → 401"
    );
    assert_eq!(
        run_one(&state, mw, Some(&tok_pending)).await,
        Expected::Status(StatusCode::FORBIDDEN),
        "Pending → 403"
    );
    assert_eq!(
        run_one(&state, mw, Some(&tok_suspended)).await,
        Expected::Status(StatusCode::UNAUTHORIZED),
        "Suspended → 401"
    );
    assert_eq!(
        run_one(&state, mw, Some(&tok_expired)).await,
        Expected::Status(StatusCode::UNAUTHORIZED),
        "Expired → 401"
    );
    assert_eq!(
        run_one(&state, mw, Some(&tok_active)).await,
        Expected::Forwarded,
        "Active → forwarded"
    );
    assert_eq!(
        run_one(&state, mw, Some(&tok_admin)).await,
        Expected::Forwarded,
        "Active-admin → forwarded"
    );

    // ---- require_auth_redirect: Expired → /portal/restore, rest → login ----
    let mw = MiddlewareKind::RequireAuthRedirect;
    assert!(
        matches!(
            run_one(&state, mw, None).await,
            Expected::Redirect(loc) if loc.starts_with("/login")
        ),
        "anonymous → login"
    );
    assert!(
        matches!(
            run_one(&state, mw, Some(&tok_pending)).await,
            Expected::Redirect(loc) if loc.starts_with("/login")
        ),
        "Pending → login"
    );
    assert!(
        matches!(
            run_one(&state, mw, Some(&tok_suspended)).await,
            Expected::Redirect(loc) if loc.starts_with("/login")
        ),
        "Suspended → login"
    );
    assert_eq!(
        run_one(&state, mw, Some(&tok_expired)).await,
        Expected::Redirect("/portal/restore"),
        "Expired → /portal/restore"
    );
    assert_eq!(
        run_one(&state, mw, Some(&tok_active)).await,
        Expected::Forwarded,
        "Active → forwarded"
    );
    assert_eq!(
        run_one(&state, mw, Some(&tok_admin)).await,
        Expected::Forwarded,
        "Active-admin → forwarded"
    );

    // ---- require_restorable: Active/Honorary/Expired forwarded ----
    let mw = MiddlewareKind::RequireRestorable;
    assert!(
        matches!(
            run_one(&state, mw, None).await,
            Expected::Redirect(loc) if loc.starts_with("/login")
        ),
        "anonymous → login"
    );
    assert!(
        matches!(
            run_one(&state, mw, Some(&tok_pending)).await,
            Expected::Redirect(loc) if loc.starts_with("/login")
        ),
        "Pending → login"
    );
    assert!(
        matches!(
            run_one(&state, mw, Some(&tok_suspended)).await,
            Expected::Redirect(loc) if loc.starts_with("/login")
        ),
        "Suspended → login"
    );
    assert_eq!(
        run_one(&state, mw, Some(&tok_expired)).await,
        Expected::Forwarded,
        "Expired → forwarded (restorable allows it)"
    );
    assert_eq!(
        run_one(&state, mw, Some(&tok_active)).await,
        Expected::Forwarded,
        "Active → forwarded"
    );
    assert_eq!(
        run_one(&state, mw, Some(&tok_admin)).await,
        Expected::Forwarded,
        "Active-admin → forwarded"
    );

    // ---- require_admin_redirect: setting OFF — non-admin → dashboard,
    //      admin → forwarded regardless of TOTP enrollment ----
    let mw = MiddlewareKind::RequireAdminRedirect;
    set_admin_totp_required(&pool, false).await;
    assert!(
        matches!(
            run_one(&state, mw, None).await,
            Expected::Redirect(loc) if loc.starts_with("/login")
        ),
        "anonymous → login"
    );
    assert!(
        matches!(
            run_one(&state, mw, Some(&tok_pending)).await,
            Expected::Redirect(loc) if loc.starts_with("/login")
        ),
        "Pending → login"
    );
    assert!(
        matches!(
            run_one(&state, mw, Some(&tok_suspended)).await,
            Expected::Redirect(loc) if loc.starts_with("/login")
        ),
        "Suspended → login"
    );
    assert!(
        matches!(
            run_one(&state, mw, Some(&tok_expired)).await,
            Expected::Redirect(loc) if loc.starts_with("/login")
        ),
        "Expired → login"
    );
    assert_eq!(
        run_one(&state, mw, Some(&tok_active)).await,
        Expected::Redirect("/portal/dashboard"),
        "Active-non-admin → /portal/dashboard"
    );
    assert_eq!(
        run_one(&state, mw, Some(&tok_admin)).await,
        Expected::Forwarded,
        "Active-admin (TOTP setting OFF) → forwarded"
    );

    // ---- require_admin_redirect: setting ON — admin without TOTP
    //      is bounced to the security page; admin with TOTP forwards ----
    set_admin_totp_required(&pool, true).await;
    assert_eq!(
        run_one(&state, mw, Some(&tok_admin)).await,
        Expected::Redirect("/portal/profile/security?reason=admin_totp_required"),
        "Active-admin without TOTP + setting ON → security page"
    );

    force_totp_enabled(&pool, admin_id).await;
    assert_eq!(
        run_one(&state, mw, Some(&tok_admin)).await,
        Expected::Forwarded,
        "Active-admin with TOTP + setting ON → forwarded"
    );
}
