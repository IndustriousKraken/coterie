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
        middleware::bot_challenge::{BotChallengeVerifier, DisabledVerifier},
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
    let auth_service = Arc::new(AuthService::new(
        pool.clone(),
        "test-session-secret-please-ignore".to_string(),
    ));
    build_app_state_with_services(pool, totp_service, auth_service, None, None).await
}

/// Variant of [`build_app_state`] for tests that need a configured
/// Stripe surface (a `StripeHandle` preloaded with a fake-gateway
/// client) and/or a custom bot-challenge verifier. `None` for either
/// keeps the default (unconfigured Stripe / `DisabledVerifier`).
pub async fn build_app_state_custom(
    pool: SqlitePool,
    stripe_handle: Option<Arc<coterie::payments::StripeHandle>>,
    verifier: Option<Arc<dyn BotChallengeVerifier>>,
) -> AppState {
    build_app_state_full(pool, stripe_handle, verifier, None).await
}

/// Variant of [`build_app_state_custom`] that also sets
/// `server.cors_origins` — for tests that assert the CORS allowlist
/// actually reaches a given public endpoint (with `None`, the layer is
/// same-origin only and emits no allow-origin header at all).
pub async fn build_app_state_full(
    pool: SqlitePool,
    stripe_handle: Option<Arc<coterie::payments::StripeHandle>>,
    verifier: Option<Arc<dyn BotChallengeVerifier>>,
    cors_origins: Option<String>,
) -> AppState {
    let crypto = Arc::new(SecretCrypto::new("test-secret-please-ignore"));
    let totp_service = Arc::new(TotpService::new(
        pool.clone(),
        crypto,
        "Coterie".to_string(),
    ));
    let auth_service = Arc::new(AuthService::new(
        pool.clone(),
        "test-session-secret-please-ignore".to_string(),
    ));
    build_app_state_with_services_and_cors(
        pool,
        totp_service,
        auth_service,
        stripe_handle,
        verifier,
        cors_origins,
    )
    .await
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
    let auth_service = Arc::new(AuthService::new(
        pool.clone(),
        "test-session-secret-please-ignore".to_string(),
    ));
    build_app_state_with_services(pool, totp_service, auth_service, None, None).await
}

/// Variant of [`build_app_state`] that lets the caller inject a custom
/// `AuthService` — useful for tests that need to force `create_session`
/// to fail (point the service at a closed pool). All other services keep
/// the main `pool` so password lookup, member repo, etc. continue to
/// work.
pub async fn build_app_state_with_auth(
    pool: SqlitePool,
    auth_service: Arc<AuthService>,
) -> AppState {
    let crypto = Arc::new(SecretCrypto::new("test-secret-please-ignore"));
    let totp_service = Arc::new(TotpService::new(
        pool.clone(),
        crypto,
        "Coterie".to_string(),
    ));
    build_app_state_with_services(pool, totp_service, auth_service, None, None).await
}

async fn build_app_state_with_services(
    pool: SqlitePool,
    totp_service: Arc<TotpService>,
    auth_service: Arc<AuthService>,
    stripe_handle_override: Option<Arc<coterie::payments::StripeHandle>>,
    verifier: Option<Arc<dyn BotChallengeVerifier>>,
) -> AppState {
    build_app_state_with_services_and_cors(
        pool,
        totp_service,
        auth_service,
        stripe_handle_override,
        verifier,
        None,
    )
    .await
}

async fn build_app_state_with_services_and_cors(
    pool: SqlitePool,
    totp_service: Arc<TotpService>,
    auth_service: Arc<AuthService>,
    stripe_handle_override: Option<Arc<coterie::payments::StripeHandle>>,
    verifier: Option<Arc<dyn BotChallengeVerifier>>,
    cors_origins: Option<String>,
) -> AppState {
    let settings = Settings {
        server: coterie::config::ServerConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            base_url: "http://127.0.0.1".to_string(),
            data_dir: "./data".to_string(),
            uploads_dir: None,
            secure_cookies: Some(false),
            cors_origins,
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

    let member_repo: Arc<dyn MemberRepository> =
        Arc::new(SqliteMemberRepository::new(pool.clone()));
    let event_repo: Arc<dyn EventRepository> = Arc::new(SqliteEventRepository::new(pool.clone()));
    let announcement_repo: Arc<dyn AnnouncementRepository> =
        Arc::new(SqliteAnnouncementRepository::new(pool.clone()));
    let payment_repo: Arc<dyn PaymentRepository> =
        Arc::new(SqlitePaymentRepository::new(pool.clone()));

    let crypto = Arc::new(SecretCrypto::new("test-secret-please-ignore"));
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
        money_limiter.clone(),
        settings.server.base_url.clone(),
        pool.clone(),
    ));

    let billing_service =
        Arc::new(service_context.billing_service(settings.server.base_url.clone()));

    // Default: the ServiceContext-owned handle stays unconfigured (no
    // Stripe surface). Tests that exercise checkout paths pass a
    // preloaded handle wired to a FakeStripeGateway.
    //
    // The override is ALSO installed into the ServiceContext-owned handle,
    // because services (e.g. `EventRegistrationService`) capture that one
    // at construction the way production does — in production both are
    // the same handle. Without this, a router test would configure Stripe
    // for handlers but leave it unconfigured for services, and a checkout
    // path through a service would answer 503.
    let stripe_handle = match stripe_handle_override {
        Some(handle) => {
            let runtime = handle.current();
            service_context
                .stripe_handle
                .install_for_test(runtime.client.clone(), runtime.webhook_dispatcher.clone());
            handle
        }
        None => service_context.stripe_handle.clone(),
    };
    let verifier = verifier.unwrap_or_else(|| Arc::new(DisabledVerifier));

    AppState::new(
        service_context,
        stripe_handle,
        billing_service,
        settings,
        verifier,
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

/// Build an `AuthService` whose backing pool has already been closed.
/// Any query through its session store (including `create_session`)
/// returns a sqlx pool-closed error — used to drive the
/// session-create-failure branch in the login handlers without
/// disturbing the shared pool that the rest of the harness needs.
pub async fn failing_auth_service() -> Arc<AuthService> {
    let bad_pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect bad pool");
    bad_pool.close().await;
    Arc::new(AuthService::new(
        bad_pool,
        "test-session-secret-please-ignore".to_string(),
    ))
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

/// Assemble the final router the way `main.rs` does: merge, then the
/// setup gate, then CSRF, then request tracing OUTERMOST.
///
/// The order is load-bearing and has regressed once already — in axum
/// 0.7 a layer applied before `Router::merge` does not reach the merged
/// routes, which is how the entire portal surface (`/login`,
/// `/forgot-password`, every `/portal/*`) ran for twenty days with no
/// request log at all. Any test that cares about a top-level layer
/// reaching the portal must build the app through here rather than
/// through `create_web_routes` alone.
pub fn merged_router(app_state: AppState) -> axum::Router {
    coterie::api::create_app(app_state.clone())
        .merge(coterie::web::create_web_routes(app_state.clone()))
        .layer(axum::middleware::from_fn_with_state(
            app_state.clone(),
            coterie::api::middleware::setup::require_setup,
        ))
        .layer(axum::middleware::from_fn_with_state(
            app_state,
            coterie::api::middleware::security::csrf_protect_unless_exempt,
        ))
        .layer(tower_http::trace::TraceLayer::new_for_http())
}

// ---------------------------------------------------------------------
// Stripe JSON fixtures
//
// The `stripe` crate's types deserialize from the real webhook wire
// shape, so every one of these carries filler the crate requires but no
// handler reads (`automatic_tax`, `custom_text`, `shipping_options`,
// `billing_details`, …). Keeping that filler in ONE place is the point:
// a crate upgrade that adds a required field is a single edit here, and
// the copies can't drift into disagreeing about what a refunded charge
// or a completed session looks like.
//
// Callers pass the parts they actually vary (ids, amounts, metadata);
// per-test wrappers that bind test-specific metadata stay in their own
// files.
// ---------------------------------------------------------------------

/// The `checkout.session` body as raw JSON, for callers that need to
/// embed it in a larger payload (a signed `stripe::Event`, say) rather
/// than deserialize it straight into a `CheckoutSession`.
pub fn checkout_session_json(
    id: &str,
    payment_intent_id: Option<&str>,
    metadata: serde_json::Value,
    amount_total: i64,
) -> serde_json::Value {
    let now = chrono::Utc::now().timestamp();
    serde_json::json!({
        "id": id,
        "object": "checkout.session",
        "livemode": false,
        "mode": "payment",
        "status": "complete",
        "payment_status": "paid",
        "created": now,
        "expires_at": now + 86400,
        "currency": "usd",
        "amount_total": amount_total,
        "amount_subtotal": amount_total,
        "metadata": metadata,
        "payment_intent": payment_intent_id,
        "automatic_tax": { "enabled": false, "liability": null, "status": null },
        "custom_fields": [],
        "custom_text": {
            "after_submit": null,
            "shipping_address": null,
            "submit": null,
            "terms_of_service_acceptance": null
        },
        "payment_method_types": ["card"],
        "shipping_options": [],
    })
}

/// A completed `checkout.session`. Only `id`, `metadata`,
/// `payment_intent` and `amount_total` are ever read by a handler; the
/// rest is the crate's required shape.
pub fn build_checkout_session(
    id: &str,
    payment_intent_id: Option<&str>,
    metadata: serde_json::Value,
    amount_total: i64,
) -> stripe::CheckoutSession {
    serde_json::from_value(checkout_session_json(
        id,
        payment_intent_id,
        metadata,
        amount_total,
    ))
    .expect("CheckoutSession from JSON")
}

/// A `charge`. `refunded` is derived from the two amounts so a fully
/// refunded charge can't be built with the flag disagreeing.
pub fn build_charge(
    id: &str,
    amount: i64,
    amount_refunded: i64,
    payment_intent: Option<&str>,
) -> stripe::Charge {
    let body = serde_json::json!({
        "id": id,
        "object": "charge",
        "amount": amount,
        "amount_captured": amount,
        "amount_refunded": amount_refunded,
        "billing_details": {
            "address": null,
            "email": null,
            "name": null,
            "phone": null,
        },
        "currency": "usd",
        "captured": true,
        "created": chrono::Utc::now().timestamp(),
        "disputed": false,
        "livemode": false,
        "paid": true,
        "refunded": amount_refunded >= amount,
        "status": "succeeded",
        "payment_intent": payment_intent,
        "metadata": {},
    });
    serde_json::from_value(body).expect("Charge from JSON")
}

/// A succeeded `payment_intent`.
pub fn build_payment_intent(
    id: &str,
    amount: i64,
    metadata: serde_json::Value,
) -> stripe::PaymentIntent {
    let body = serde_json::json!({
        "id": id,
        "object": "payment_intent",
        "amount": amount,
        "amount_received": amount,
        "amount_capturable": 0,
        "currency": "usd",
        "status": "succeeded",
        "livemode": false,
        "created": chrono::Utc::now().timestamp(),
        "metadata": metadata,
        "capture_method": "automatic",
        "confirmation_method": "automatic",
        "payment_method_types": ["card"],
    });
    serde_json::from_value(body).expect("PaymentIntent from JSON")
}

/// A canceled `subscription` — the shape `customer.subscription.deleted`
/// carries.
pub fn build_subscription(id: &str, customer_id: &str) -> stripe::Subscription {
    let now = chrono::Utc::now().timestamp();
    let body = serde_json::json!({
        "id": id,
        "object": "subscription",
        "customer": customer_id,
        "status": "canceled",
        "created": now,
        "current_period_start": now,
        "current_period_end": now + 86400,
        "start_date": now,
        "livemode": false,
        "cancel_at_period_end": false,
        "collection_method": "charge_automatically",
        "automatic_tax": { "enabled": false, "liability": null },
        "billing_cycle_anchor": now,
        "currency": "usd",
        "metadata": {},
        "items": {
            "object": "list",
            "data": [],
            "has_more": false,
            "total_count": 0,
            "url": "/v1/subscription_items"
        },
    });
    serde_json::from_value(body).expect("Subscription from JSON")
}

/// An `invoice`. `next_payment_attempt` is only set when supplied — its
/// absence is what tells the dues path a dunning cycle is over.
#[allow(clippy::too_many_arguments)]
pub fn build_invoice(
    id: &str,
    customer_id: &str,
    subscription_id: &str,
    amount_paid: i64,
    status: &str,
    attempt_count: u64,
    next_payment_attempt: Option<i64>,
) -> stripe::Invoice {
    let now = chrono::Utc::now().timestamp();
    let mut body = serde_json::json!({
        "id": id,
        "object": "invoice",
        "customer": customer_id,
        "subscription": subscription_id,
        "amount_paid": amount_paid,
        "amount_due": amount_paid,
        "amount_remaining": 0,
        "currency": "usd",
        "status": status,
        "attempt_count": attempt_count,
        "livemode": false,
        "created": now,
        "period_start": now,
        "period_end": now + 86400 * 60,
    });
    if let Some(ts) = next_payment_attempt {
        body["next_payment_attempt"] = serde_json::json!(ts);
    }
    serde_json::from_value(body).expect("Invoice from JSON")
}
