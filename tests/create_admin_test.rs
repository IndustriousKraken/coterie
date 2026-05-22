//! Tests for the `create_admin` bootstrap CLI and the tightened GET
//! /setup behaviour that ships alongside it.
//!
//! The CLI tests exercise `coterie::admin_cli::run_with_pool` directly
//! against an in-memory SQLite pool — same pattern as the other
//! repository-level integration tests in this crate. The setup-redirect
//! test boots the merged router and drives `GET /setup` end-to-end.

use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use coterie::{
    admin_cli::{self, Cli, CreateAdminOutcome, PasswordSource},
    api::{
        middleware::bot_challenge::DisabledVerifier,
        state::{AppState, MoneyLimiter, RateLimiter},
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

// ----------------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------------

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

fn cli_with_inline_password(email: &str, username: &str, name: &str, password: &str) -> Cli {
    Cli {
        email: email.to_string(),
        username: username.to_string(),
        full_name: name.to_string(),
        password: PasswordSource {
            password: Some(password.to_string()),
            password_file: None,
        },
    }
}

fn cli_with_password_file(email: &str, username: &str, name: &str, path: PathBuf) -> Cli {
    Cli {
        email: email.to_string(),
        username: username.to_string(),
        full_name: name.to_string(),
        password: PasswordSource {
            password: None,
            password_file: Some(path),
        },
    }
}

// ----------------------------------------------------------------------
// Section 4.2: happy path
// ----------------------------------------------------------------------

#[tokio::test]
async fn happy_path_creates_admin() {
    let pool = fresh_pool().await;
    let cli = cli_with_inline_password(
        "founder@example.com",
        "founder",
        "Founder Person",
        "BootstrapPass1",
    );

    let outcome = admin_cli::run_with_pool(&cli, &pool)
        .await
        .expect("run should succeed on a fresh DB");
    let id = match outcome {
        CreateAdminOutcome::Created(id) => id,
        CreateAdminOutcome::AlreadyExists => panic!("fresh DB should not refuse"),
    };

    // Row exists with the right fields.
    let row: (String, String, String, String, i64, String) = sqlx::query_as(
        "SELECT email, username, full_name, status, is_admin, password_hash \
         FROM members WHERE id = ?",
    )
    .bind(id.to_string())
    .fetch_one(&pool)
    .await
    .expect("select admin row");

    assert_eq!(row.0, "founder@example.com");
    assert_eq!(row.1, "founder");
    assert_eq!(row.2, "Founder Person");
    assert_eq!(row.3, "Active");
    assert_eq!(row.4, 1, "is_admin must be 1");
    assert!(
        AuthService::verify_password("BootstrapPass1", &row.5)
            .await
            .expect("verify password"),
        "stored hash must verify against the supplied password"
    );

    // email_verified_at must be set (we treat the bootstrap admin as
    // implicitly verified, matching the /setup POST handler).
    let verified: Option<chrono::NaiveDateTime> =
        sqlx::query_scalar("SELECT email_verified_at FROM members WHERE id = ?")
            .bind(id.to_string())
            .fetch_one(&pool)
            .await
            .expect("select email_verified_at");
    assert!(verified.is_some(), "email_verified_at must be set");
}

// ----------------------------------------------------------------------
// Section 4.3: idempotent refusal
// ----------------------------------------------------------------------

#[tokio::test]
async fn refuses_when_admin_exists() {
    let pool = fresh_pool().await;

    // Pre-seed an admin via the repository (mirrors how the manual
    // /setup form would have populated it).
    let repo = SqliteMemberRepository::new(pool.clone());
    let pre = repo
        .create(CreateMemberRequest {
            email: "preexisting@example.com".to_string(),
            username: "preadmin".to_string(),
            full_name: "Pre-existing Admin".to_string(),
            password: "PreAdminPass1".to_string(),
            membership_type_id: None,
            ..Default::default()
        })
        .await
        .expect("seed pre-existing admin");
    repo.update(
        pre.id,
        UpdateMemberRequest {
            status: Some(MemberStatus::Active),
            bypass_dues: Some(true),
            ..Default::default()
        },
    )
    .await
    .expect("activate pre-existing admin");
    repo.set_admin(pre.id, true)
        .await
        .expect("promote pre-existing admin");

    let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM members")
        .fetch_one(&pool)
        .await
        .expect("count members before");

    let cli = cli_with_inline_password(
        "second@example.com",
        "second",
        "Second Person",
        "SecondPass1A",
    );
    let outcome = admin_cli::run_with_pool(&cli, &pool)
        .await
        .expect("run returns Ok(AlreadyExists), not Err");

    assert!(
        matches!(outcome, CreateAdminOutcome::AlreadyExists),
        "expected AlreadyExists, got {:?}",
        outcome,
    );

    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM members")
        .fetch_one(&pool)
        .await
        .expect("count members after");
    assert_eq!(before, after, "no second insert should have happened");
}

// ----------------------------------------------------------------------
// Section 4.4: password file trims trailing whitespace
// ----------------------------------------------------------------------

#[tokio::test]
async fn password_file_strips_trailing_newline() {
    let pool = fresh_pool().await;

    // tempfile-free: just put the file under target/ in a uniquely-named
    // subdir so parallel test runs don't collide.
    let tmpdir = std::env::temp_dir().join(format!(
        "coterie-create-admin-test-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&tmpdir).expect("create tmpdir");
    let path = tmpdir.join("pw");
    std::fs::write(&path, b"FilePassword1\n").expect("write pw file");

    let cli = cli_with_password_file(
        "fileadmin@example.com",
        "fileadmin",
        "File Admin",
        path.clone(),
    );
    let outcome = admin_cli::run_with_pool(&cli, &pool)
        .await
        .expect("run with password file");
    let id = match outcome {
        CreateAdminOutcome::Created(id) => id,
        CreateAdminOutcome::AlreadyExists => panic!("fresh DB should not refuse"),
    };

    let hash: String = sqlx::query_scalar("SELECT password_hash FROM members WHERE id = ?")
        .bind(id.to_string())
        .fetch_one(&pool)
        .await
        .expect("select hash");

    assert!(
        AuthService::verify_password("FilePassword1", &hash)
            .await
            .expect("verify"),
        "hash must verify against the trimmed password"
    );
    assert!(
        !AuthService::verify_password("FilePassword1\n", &hash)
            .await
            .expect("verify (with newline)"),
        "hash must NOT verify against the password with the trailing newline"
    );

    // Cleanup; ignore errors since /tmp gets swept anyway.
    let _ = std::fs::remove_dir_all(&tmpdir);
}

// ----------------------------------------------------------------------
// Section 4.5: GET /setup redirects when admin exists
// ----------------------------------------------------------------------
//
// Boots the merged router (same shape as `main.rs` minus the CSRF
// layer, matching the existing setup_redirect_test.rs harness), pre-
// creates an admin, and asserts GET /setup returns 303 → /login.

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
    let money_limiter =
        MoneyLimiter(RateLimiter::new(10, std::time::Duration::from_secs(60)));

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
        None,
        money_limiter.clone(),
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
        money_limiter,
    )
}

#[tokio::test]
async fn setup_get_redirects_when_admin_exists() {
    let pool = fresh_pool().await;

    // Bootstrap via the CLI under test — this proves the binary's
    // output is observable by the web layer's setup_page check.
    let cli = cli_with_inline_password(
        "wizard@example.com",
        "wizardadmin",
        "Wizard Admin",
        "WizardBootPass1",
    );
    let outcome = admin_cli::run_with_pool(&cli, &pool)
        .await
        .expect("create admin via CLI");
    assert!(
        matches!(outcome, CreateAdminOutcome::Created(_)),
        "expected Created"
    );

    let state = build_app_state(pool).await;
    let app = coterie::web::create_web_routes(state.clone());

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/setup")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::SEE_OTHER,
        "GET /setup should redirect with 303 when an admin already exists"
    );
    let loc = resp
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(loc, "/login", "redirect target must be /login");
}

#[tokio::test]
async fn setup_get_renders_form_when_no_admin() {
    // Sanity-check the negative side: a fresh DB still renders the
    // setup form, so we haven't accidentally bricked first-boot.
    let pool = fresh_pool().await;
    let state = build_app_state(pool).await;
    let app = coterie::web::create_web_routes(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/setup")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "GET /setup on a fresh DB should render the form (HTTP 200)"
    );
}
