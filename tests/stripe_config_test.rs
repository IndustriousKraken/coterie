//! Tests for portal-configurable Stripe: DB-backed config with
//! write-only secrets, the hot-swappable `StripeHandle` (rebuild on
//! save, blank secret → unconfigured), the one-time `.env` seed, and
//! the removal of the dead `integrations.stripe.*` rows.
//!
//! Run: cargo test --features test-utils --test stripe_config_test

use std::sync::Arc;

use coterie::{
    auth::SecretCrypto,
    config::StripeConfig,
    integrations::IntegrationManager,
    payments::{
        fake_gateway::FakeStripeGateway,
        gateway::StripeGateway,
        runtime::{GatewayFactory, SYSTEM_ACTOR},
        StripeHandle,
    },
    repository::{
        MemberRepository, PaymentRepository, ProcessedEventsRepository, SqliteMemberRepository,
        SqliteMembershipTypeRepository, SqlitePaymentRepository, SqliteProcessedEventsRepository,
    },
    service::{
        membership_type_service::MembershipTypeService,
        settings_service::{stripe_keys, SettingsService, UpdateStripeConfig},
    },
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use sqlx::SqlitePool;
use uuid::Uuid;

mod common;
use common::fresh_pool;

fn settings_service(pool: &SqlitePool) -> Arc<SettingsService> {
    let crypto = Arc::new(SecretCrypto::new("test-secret-please-ignore"));
    Arc::new(SettingsService::new(pool.clone(), crypto))
}

/// Build a StripeHandle whose gateways are a shared `FakeStripeGateway`,
/// so `rebuild()` exercises the whole path with no real Stripe calls.
fn build_handle(pool: &SqlitePool, settings: &Arc<SettingsService>) -> Arc<StripeHandle> {
    let fake = Arc::new(FakeStripeGateway::new());
    let factory: GatewayFactory = Arc::new(move |_key| fake.clone() as Arc<dyn StripeGateway>);

    let payment_repo: Arc<dyn PaymentRepository> =
        Arc::new(SqlitePaymentRepository::new(pool.clone()));
    let member_repo: Arc<dyn MemberRepository> =
        Arc::new(SqliteMemberRepository::new(pool.clone()));
    let processed_events_repo: Arc<dyn ProcessedEventsRepository> =
        Arc::new(SqliteProcessedEventsRepository::new(pool.clone()));
    let mt_service = Arc::new(MembershipTypeService::new(Arc::new(
        SqliteMembershipTypeRepository::new(pool.clone()),
    )));
    let integrations = Arc::new(IntegrationManager::new());

    Arc::new(StripeHandle::with_gateway_factory(
        factory,
        settings.clone(),
        payment_repo,
        member_repo,
        processed_events_repo,
        mt_service,
        integrations,
    ))
}

/// Produce a valid `Stripe-Signature` header for `payload` under
/// `secret`, matching async-stripe's scheme: HMAC-SHA256 over
/// `"{t}.{payload}"` with the secret bytes as key, hex-encoded as `v1`.
fn sign(secret: &str, payload: &str, ts: i64) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(format!("{}.{}", ts, payload).as_bytes());
    let hex = hex::encode(mac.finalize().into_bytes());
    format!("t={},v1={}", ts, hex)
}

/// A minimal but valid `stripe::Event` JSON. `type` is "ping" →
/// `EventType::Unknown`, so the dispatcher's `_` arm returns Ok(())
/// once the signature verifies — keeping this focused on the secret.
fn event_payload(event_id: &str) -> String {
    let ts = chrono::Utc::now().timestamp();
    serde_json::json!({
        "id": event_id,
        "object": "event",
        "type": "ping",
        "created": ts,
        "livemode": false,
        "pending_webhooks": 0,
        // A full checkout.session object (the field set proven to
        // deserialize in stripe_webhook_test). The event type is "ping"
        // (Unknown) so the object is never inspected — it just has to
        // parse so `construct_event` reaches the `_` dispatch arm.
        "data": {
            "object": {
                "id": "cs_probe",
                "object": "checkout.session",
                "livemode": false,
                "mode": "payment",
                "status": "complete",
                "payment_status": "paid",
                "created": ts,
                "expires_at": ts + 86400,
                "currency": "usd",
                "amount_total": 5000,
                "amount_subtotal": 5000,
                "metadata": {},
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
            }
        }
    })
    .to_string()
}

// ---------------------------------------------------------------------
// 5.1 — UpdateStripeConfig secret semantics
// ---------------------------------------------------------------------

#[tokio::test]
async fn blank_secret_keeps_existing_nonempty_replaces() {
    let pool = fresh_pool().await;
    let svc = settings_service(&pool);
    let actor = Uuid::nil();

    // Store an initial secret + webhook secret.
    svc.update_stripe_config(
        UpdateStripeConfig {
            enabled: true,
            publishable_key: "pk_test_1".into(),
            success_url: String::new(),
            cancel_url: String::new(),
            secret_key: Some("sk_test_first".into()),
            webhook_secret: Some("whsec_first".into()),
        },
        actor,
    )
    .await
    .unwrap();

    // A save with None secrets (blank form) must KEEP both.
    svc.update_stripe_config(
        UpdateStripeConfig {
            enabled: true,
            publishable_key: "pk_test_2".into(),
            success_url: String::new(),
            cancel_url: String::new(),
            secret_key: None,
            webhook_secret: None,
        },
        actor,
    )
    .await
    .unwrap();

    let cfg = svc.get_stripe_config().await.unwrap();
    assert_eq!(
        cfg.secret_key, "sk_test_first",
        "blank secret must keep stored"
    );
    assert_eq!(
        cfg.webhook_secret, "whsec_first",
        "blank webhook must keep stored"
    );
    assert_eq!(
        cfg.publishable_key, "pk_test_2",
        "non-secret fields still update"
    );

    // A save with Some(nonempty) must REPLACE.
    svc.update_stripe_config(
        UpdateStripeConfig {
            enabled: true,
            publishable_key: "pk_test_2".into(),
            success_url: String::new(),
            cancel_url: String::new(),
            secret_key: Some("sk_test_second".into()),
            webhook_secret: None,
        },
        actor,
    )
    .await
    .unwrap();
    let cfg = svc.get_stripe_config().await.unwrap();
    assert_eq!(
        cfg.secret_key, "sk_test_second",
        "nonempty secret must replace"
    );
    assert_eq!(
        cfg.webhook_secret, "whsec_first",
        "untouched webhook still kept"
    );

    // Secrets are stored as ciphertext, not plaintext.
    let raw: String = sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
        .bind(stripe_keys::SECRET_KEY)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_ne!(raw, "sk_test_second", "secret must be encrypted at rest");
    assert!(!raw.is_empty());
}

#[tokio::test]
async fn blank_webhook_secret_resolves_to_unconfigured() {
    let pool = fresh_pool().await;
    let svc = settings_service(&pool);
    let handle = build_handle(&pool, &svc);

    // Enabled with a secret key but a BLANK webhook secret.
    svc.update_stripe_config(
        UpdateStripeConfig {
            enabled: true,
            publishable_key: "pk".into(),
            success_url: String::new(),
            cancel_url: String::new(),
            secret_key: Some("sk_test_x".into()),
            webhook_secret: Some("   ".into()), // whitespace → blank
        },
        Uuid::nil(),
    )
    .await
    .unwrap();

    handle.rebuild().await;
    let rt = handle.current();
    assert!(rt.client.is_none(), "blank webhook secret → no client");
    assert!(
        rt.webhook_dispatcher.is_none(),
        "blank webhook secret → no dispatcher (webhook 503)"
    );
}

// ---------------------------------------------------------------------
// 5.2 — hot reload: save valid → client built + webhook verifies with
//        the new secret; save blank secret → disabled (webhook 503).
// ---------------------------------------------------------------------

#[tokio::test]
async fn save_rebuilds_client_and_new_secret_verifies_without_restart() {
    let pool = fresh_pool().await;
    let svc = settings_service(&pool);
    let handle = build_handle(&pool, &svc);

    // Nothing configured yet → unconfigured.
    handle.rebuild().await;
    assert!(handle.current().webhook_dispatcher.is_none());

    // Save a valid config with webhook secret A.
    let secret_a = "whsec_AAA_secret";
    svc.update_stripe_config(
        UpdateStripeConfig {
            enabled: true,
            publishable_key: "pk_test".into(),
            success_url: String::new(),
            cancel_url: String::new(),
            secret_key: Some("sk_test_live".into()),
            webhook_secret: Some(secret_a.into()),
        },
        Uuid::nil(),
    )
    .await
    .unwrap();
    handle.rebuild().await;

    let rt = handle.current();
    assert!(rt.client.is_some(), "valid config builds a client");
    let dispatcher = rt
        .webhook_dispatcher
        .clone()
        .expect("valid config builds a webhook dispatcher");

    // Build the billing service the dispatcher's handlers may need.
    let billing = build_billing(&pool, &svc).await;

    let ts = chrono::Utc::now().timestamp();
    let payload = event_payload("evt_hotreload_ok");

    // Signed with the NEW secret → verifies (gets past signature check;
    // an unhandled "ping" event returns Ok).
    let good_sig = sign(secret_a, &payload, ts);
    dispatcher
        .handle_webhook(&payload, &good_sig, &billing)
        .await
        .expect("webhook signed with the saved secret must verify");

    // Signed with a DIFFERENT (old) secret → rejected as invalid.
    let bad_sig = sign("whsec_OLD_secret", &payload, ts);
    let err = dispatcher
        .handle_webhook(&payload, &bad_sig, &billing)
        .await
        .expect_err("webhook signed with the wrong secret must be rejected");
    assert!(
        err.to_string().contains("Invalid signature"),
        "expected an invalid-signature error, got: {}",
        err
    );

    // Now save a BLANK secret key → Stripe disabled, webhook → 503.
    svc.update_stripe_config(
        UpdateStripeConfig {
            enabled: true,
            publishable_key: "pk_test".into(),
            success_url: String::new(),
            cancel_url: String::new(),
            secret_key: Some(String::new()), // cleared
            webhook_secret: None,
        },
        Uuid::nil(),
    )
    .await
    .unwrap();
    handle.rebuild().await;
    let rt = handle.current();
    assert!(rt.client.is_none(), "blank secret disables the client");
    assert!(
        rt.webhook_dispatcher.is_none(),
        "blank secret → webhook dispatcher gone (endpoint 503) — no restart",
    );
}

// ---------------------------------------------------------------------
// 5.3 — .env seeds the DB once, then the DB is authoritative
// ---------------------------------------------------------------------

#[tokio::test]
async fn seed_from_env_populates_empty_db_then_env_ignored() {
    let pool = fresh_pool().await;
    let svc = settings_service(&pool);

    let env = StripeConfig {
        publishable_key: Some("pk_env".into()),
        secret_key: Some("sk_env_secret".into()),
        webhook_secret: Some("whsec_env".into()),
        enabled: true,
    };

    // DB empty + env present → seeded.
    let seeded = svc.seed_stripe_from_env(&env, SYSTEM_ACTOR).await.unwrap();
    assert!(seeded, "empty DB + env present must seed");

    let cfg = svc.get_stripe_config().await.unwrap();
    assert!(cfg.enabled);
    assert_eq!(cfg.secret_key, "sk_env_secret");
    assert_eq!(cfg.webhook_secret, "whsec_env");
    assert_eq!(cfg.publishable_key, "pk_env");

    // DB now present → a second call with a DIFFERENT env is ignored.
    let env2 = StripeConfig {
        publishable_key: Some("pk_DIFFERENT".into()),
        secret_key: Some("sk_DIFFERENT".into()),
        webhook_secret: Some("whsec_DIFFERENT".into()),
        enabled: true,
    };
    let seeded2 = svc.seed_stripe_from_env(&env2, SYSTEM_ACTOR).await.unwrap();
    assert!(!seeded2, "DB present → must NOT re-seed from env");

    let cfg = svc.get_stripe_config().await.unwrap();
    assert_eq!(
        cfg.secret_key, "sk_env_secret",
        "portal/DB value wins over .env"
    );
}

#[tokio::test]
async fn seed_noop_when_env_absent() {
    let pool = fresh_pool().await;
    let svc = settings_service(&pool);

    let env = StripeConfig::default(); // no keys
    let seeded = svc.seed_stripe_from_env(&env, SYSTEM_ACTOR).await.unwrap();
    assert!(!seeded, "no env secret → nothing to seed");
    assert!(!svc.has_stripe_config().await, "DB stays pristine");
}

// ---------------------------------------------------------------------
// 5.4 — the generic settings page no longer carries integrations.stripe.*
// ---------------------------------------------------------------------

#[tokio::test]
async fn generic_settings_page_has_no_stripe_rows() {
    let pool = fresh_pool().await;

    // The dead rows were removed by migration.
    let dead: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM app_settings WHERE key LIKE 'integrations.stripe.%'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(dead, 0, "integrations.stripe.* rows must be gone");

    // The generic settings page renders the `integrations` category; it
    // must contain nothing Stripe. (The new stripe.* rows live under the
    // `stripe` category, which the generic page does not render.)
    let svc = settings_service(&pool);
    let categories = svc.get_all_settings().await.unwrap();
    if let Some(integrations) = categories.iter().find(|c| c.name == "integrations") {
        assert!(
            !integrations
                .settings
                .iter()
                .any(|s| s.key.contains("stripe")),
            "no stripe key may appear in the rendered 'integrations' category",
        );
    }
    for s in categories.iter().flat_map(|c| &c.settings) {
        if s.key.starts_with("stripe.") {
            assert_eq!(
                s.category, "stripe",
                "stripe.* rows must be in the hidden 'stripe' category"
            );
        }
    }
}

// ---------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------

async fn build_billing(
    pool: &SqlitePool,
    settings: &Arc<SettingsService>,
) -> coterie::service::billing_service::BillingService {
    use coterie::{
        email::LogSender,
        repository::{
            SqliteEventRepository, SqliteSavedCardRepository, SqliteScheduledPaymentRepository,
        },
    };

    let payment_repo: Arc<dyn PaymentRepository> =
        Arc::new(SqlitePaymentRepository::new(pool.clone()));
    let member_repo: Arc<dyn MemberRepository> =
        Arc::new(SqliteMemberRepository::new(pool.clone()));
    let scheduled_repo = Arc::new(SqliteScheduledPaymentRepository::new(pool.clone()));
    let saved_card_repo = Arc::new(SqliteSavedCardRepository::new(pool.clone()));
    let event_repo: Arc<dyn coterie::repository::EventRepository> =
        Arc::new(SqliteEventRepository::new(pool.clone()));
    let mt_service = Arc::new(MembershipTypeService::new(Arc::new(
        SqliteMembershipTypeRepository::new(pool.clone()),
    )));
    let email_sender = Arc::new(LogSender::new("t@example.com".into(), "T".into()));
    let integrations = Arc::new(IntegrationManager::new());

    coterie::service::billing_service::BillingService::new(
        scheduled_repo,
        payment_repo,
        saved_card_repo,
        member_repo,
        event_repo,
        mt_service,
        settings.clone(),
        email_sender,
        integrations,
        None,
        "http://localhost:3000".to_string(),
        pool.clone(),
    )
}
