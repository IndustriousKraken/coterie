//! Shared scaffolding for integration tests under `tests/`.
//!
//! Rust compiles every `.rs` file directly in `tests/` as its own
//! test binary, so there's no implicit way to share helpers across
//! them. Placing this module under `tests/common/` (as `mod.rs`)
//! prevents Cargo from compiling it as a standalone binary; each
//! test file pulls it in with `mod common;` near the top.
//!
//! Only helpers duplicated across multiple integration tests live
//! here — single-test helpers stay in their owning file.
//!
//! `dead_code` is silenced because each test binary inlines this
//! module independently; an item used by some tests but not others
//! would otherwise trip the lint in the binaries that don't use it.

#![allow(dead_code)]

use std::sync::Arc;

use coterie::{
    api::{
        middleware::bot_challenge::DisabledVerifier,
        state::{AppState, MoneyLimiter, RateLimiter},
    },
    auth::{AuthService, CsrfService, PendingLoginService, SecretCrypto, TotpService},
    config::Settings,
    domain::CreateMemberRequest,
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
use uuid::Uuid;

/// Fresh in-memory SQLite pool with all migrations applied and
/// `PRAGMA foreign_keys = ON` enforced on every connection. Pool is
/// pinned to a single connection because `sqlite::memory:` databases
/// are connection-private — multiple connections in the same pool
/// would each see an empty schema.
pub async fn fresh_pool() -> SqlitePool {
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

/// Variant of [`fresh_pool`] used by tests that want migrations applied
/// but **no** seeded `event_types` / `announcement_types` rows (so the
/// test can assert on a known-empty starting state). Returns
/// `anyhow::Result` because its callers chain `?` through a shared
/// fallible test signature.
pub async fn fresh_pool_no_seeded_basic_types() -> anyhow::Result<SqlitePool> {
    let pool = fresh_pool().await;
    sqlx::query("DELETE FROM event_types")
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM announcement_types")
        .execute(&pool)
        .await?;
    Ok(pool)
}

/// Build a router-level `AppState` for integration tests that exercise
/// middleware / handler stacks. Uses the same dummy `Settings`
/// (loopback host, 1-connection in-memory DB, test-only secrets) every
/// caller was already constructing inline; `stripe_client` is `None`
/// because no router-test path needs the real Stripe surface.
pub async fn build_app_state(pool: SqlitePool) -> AppState {
    let crypto = Arc::new(SecretCrypto::new("test-secret-please-ignore"));
    let totp_service = Arc::new(TotpService::new(
        pool.clone(),
        crypto,
        "Coterie".to_string(),
    ));
    build_app_state_with_totp(pool, totp_service).await
}

/// Variant of [`build_app_state`] that lets the caller inject a custom
/// `TotpService` — useful for tests that need to force the enrollment
/// query to fail (point the service at a closed pool). All other
/// services keep the main `pool` so password lookup, session create,
/// member repo, etc. continue to work.
pub async fn build_app_state_with_totp(
    pool: SqlitePool,
    totp_service: Arc<TotpService>,
) -> AppState {
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
    let event_repo: Arc<dyn EventRepository> = Arc::new(SqliteEventRepository::new(pool.clone()));
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
    let pending_login_service = Arc::new(PendingLoginService::new(pool.clone()));
    let settings_service = Arc::new(SettingsService::new(pool.clone(), crypto));

    let email_sender = Arc::new(LogSender::new(
        "test@example.com".to_string(),
        "Test".to_string(),
    ));
    let integration_manager = Arc::new(IntegrationManager::new());

    let money_limiter = MoneyLimiter(RateLimiter::new(10, std::time::Duration::from_secs(60)));

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

    let billing_service =
        Arc::new(service_context.billing_service(None, settings.server.base_url.clone()));

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

/// Build a `TotpService` whose backing pool has already been closed.
/// Any query through it (including `is_enabled`) will return a sqlx
/// pool-closed error — the test driver uses this to force the
/// fail-closed branch in the login handlers without dropping the
/// shared pool that the rest of the harness needs.
pub async fn failing_totp_service() -> Arc<TotpService> {
    let bad_pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect bad pool");
    bad_pool.close().await;
    let crypto = Arc::new(SecretCrypto::new("test-secret-please-ignore"));
    Arc::new(TotpService::new(bad_pool, crypto, "Coterie".to_string()))
}

/// Insert a fresh test member through `SqliteMemberRepository::create`
/// and return its id. Email / username are randomized so successive
/// calls in the same pool don't trip the uniqueness constraints.
pub async fn make_member(pool: &SqlitePool) -> Uuid {
    let repo = SqliteMemberRepository::new(pool.clone());
    let member = repo
        .create(CreateMemberRequest {
            email: format!("u-{}@example.com", Uuid::new_v4()),
            username: format!("u_{}", Uuid::new_v4().simple()),
            full_name: "Test User".to_string(),
            password: "p4ssword_long_enough".to_string(),
            membership_type_id: None,
            ..Default::default()
        })
        .await
        .expect("create member");
    member.id
}

/// Variant of [`make_member`] that also returns the freshly generated
/// email — used by tests that need to drive flows keyed on the email
/// (e.g. TOTP enrollment account names).
pub async fn make_member_with_email(pool: &SqlitePool) -> (Uuid, String) {
    let repo = SqliteMemberRepository::new(pool.clone());
    let member = repo
        .create(CreateMemberRequest {
            email: format!("u-{}@example.com", Uuid::new_v4()),
            username: format!("u_{}", Uuid::new_v4().simple()),
            full_name: "Test User".to_string(),
            password: "p4ssword_long_enough".to_string(),
            membership_type_id: None,
            ..Default::default()
        })
        .await
        .expect("create member");
    let email = member.email.clone();
    (member.id, email)
}
