//! Bot-challenge configured from DB-backed `app_settings` (migration 041)
//! and read live per request by `DynamicBotChallengeVerifier`.
//!
//! These exercise the settings→verifier path directly (no network): the
//! `Missing`-token branch returns before any siteverify POST, so
//! "verification ran" is provable without a live provider.

mod common;

use std::sync::Arc;

use coterie::{
    api::middleware::bot_challenge::{
        BotChallengeVerifier, DynamicBotChallengeVerifier, VerifyError,
    },
    auth::SecretCrypto,
    domain::UpdateSettingRequest,
    service::settings_service::{bot_challenge_keys, SettingsService},
};
use sqlx::SqlitePool;
use uuid::Uuid;

async fn setup() -> (Arc<SettingsService>, Uuid, SqlitePool) {
    let pool = common::fresh_pool().await;
    let crypto = Arc::new(SecretCrypto::new("test-secret-please-ignore"));
    let settings = Arc::new(SettingsService::new(pool.clone(), crypto));
    let member = common::make_member(&pool).await;
    (settings, member, pool)
}

async fn set(settings: &SettingsService, member: Uuid, key: &str, value: &str) {
    settings
        .update_setting(
            key,
            UpdateSettingRequest {
                value: value.to_string(),
                reason: None,
            },
            member,
        )
        .await
        .expect("update setting");
}

fn dynamic(settings: Arc<SettingsService>) -> DynamicBotChallengeVerifier {
    DynamicBotChallengeVerifier::new(settings, reqwest::Client::new())
}

/// 4.1 — provider `disabled` (the seeded default) passes the request
/// through even with no token.
#[tokio::test]
async fn disabled_provider_passes_through() {
    let (settings, _member, _pool) = setup().await;
    let verifier = dynamic(settings);

    assert!(
        verifier.verify("public/signup", None, None).await.is_ok(),
        "disabled provider must let unsigned requests through"
    );
}

/// 4.2 — provider `turnstile` with a stored secret runs verification;
/// a missing token fails closed (the handler maps `Err` to 403).
#[tokio::test]
async fn turnstile_provider_fails_closed_on_missing_token() {
    let (settings, member, _pool) = setup().await;
    set(&settings, member, bot_challenge_keys::PROVIDER, "turnstile").await;
    set(
        &settings,
        member,
        bot_challenge_keys::SECRET_KEY,
        "1x0000000000000000000000000000000AA",
    )
    .await;

    let verifier = dynamic(settings);
    let err = verifier
        .verify("public/signup", None, None)
        .await
        .expect_err("turnstile with no token must fail closed");
    assert!(matches!(err, VerifyError::Missing));
}

/// 4.3 — the secret is encrypted at rest: the stored value is ciphertext
/// (not the plaintext key), and `get_bot_challenge_config` decrypts it
/// back for the siteverify call.
#[tokio::test]
async fn secret_round_trips_encrypted() {
    let (settings, member, pool) = setup().await;
    let plaintext = "super-secret-turnstile-key";
    set(&settings, member, bot_challenge_keys::SECRET_KEY, plaintext).await;

    let stored: (String,) = sqlx::query_as("SELECT value FROM app_settings WHERE key = ?")
        .bind(bot_challenge_keys::SECRET_KEY)
        .fetch_one(&pool)
        .await
        .expect("read stored secret");
    assert!(!stored.0.is_empty(), "secret should be stored");
    assert_ne!(stored.0, plaintext, "secret must be encrypted at rest");

    let cfg = settings
        .get_bot_challenge_config()
        .await
        .expect("decrypt config");
    assert_eq!(
        cfg.secret_key, plaintext,
        "secret must decrypt for siteverify"
    );
}

/// 4.4 — flipping the provider setting changes behavior on the SAME
/// verifier instance (no reconstruction): the verifier reads settings live.
#[tokio::test]
async fn provider_change_takes_effect_without_reconstruction() {
    let (settings, member, _pool) = setup().await;
    let verifier = dynamic(settings.clone());

    // Seeded default is `disabled` → passes through.
    assert!(verifier.verify("public/signup", None, None).await.is_ok());

    // Flip to turnstile; the same verifier now fails closed on no token.
    set(&settings, member, bot_challenge_keys::PROVIDER, "turnstile").await;
    assert!(matches!(
        verifier
            .verify("public/signup", None, None)
            .await
            .expect_err("turnstile fails closed"),
        VerifyError::Missing
    ));

    // Flip back to disabled; behavior reverts, still the same instance.
    set(&settings, member, bot_challenge_keys::PROVIDER, "disabled").await;
    assert!(verifier.verify("public/signup", None, None).await.is_ok());
}
