//! Tests for portal-configurable UniFi: DB-backed config with a
//! write-only password, config read at operation time, the one-time
//! `.env` seed, the test-connection path (reports success/failure without
//! persisting credentials), and the removal of the dead
//! `integrations.unifi.enabled` row.
//!
//! Run: cargo test --features test-utils --test unifi_config_test

use std::sync::Arc;

use coterie::{
    auth::SecretCrypto,
    config::UnifiConfig,
    integrations::unifi,
    payments::runtime::SYSTEM_ACTOR,
    service::settings_service::{unifi_keys, SettingsService, UpdateUnifiConfig},
};
use sqlx::SqlitePool;
use uuid::Uuid;

mod common;
use common::fresh_pool;

fn settings_service(pool: &SqlitePool) -> Arc<SettingsService> {
    let crypto = Arc::new(SecretCrypto::new("test-secret-please-ignore"));
    Arc::new(SettingsService::new(pool.clone(), crypto))
}

fn base_update(password: Option<String>) -> UpdateUnifiConfig {
    UpdateUnifiConfig {
        enabled: true,
        controller_url: "https://controller.example".into(),
        username: "svc".into(),
        site_id: "default".into(),
        password,
    }
}

// ---------------------------------------------------------------------
// 5.1 — UpdateUnifiConfig password semantics
// ---------------------------------------------------------------------

#[tokio::test]
async fn blank_password_keeps_existing_nonempty_replaces() {
    let pool = fresh_pool().await;
    let svc = settings_service(&pool);
    let actor = Uuid::nil();

    // Store an initial password.
    svc.update_unifi_config(base_update(Some("first-pw".into())), actor)
        .await
        .unwrap();

    // A save with None (blank form) must KEEP the stored password while
    // still updating the non-secret fields.
    svc.update_unifi_config(
        UpdateUnifiConfig {
            controller_url: "https://changed.example".into(),
            password: None,
            ..base_update(None)
        },
        actor,
    )
    .await
    .unwrap();

    let cfg = svc.get_unifi_config().await.unwrap();
    assert_eq!(cfg.password, "first-pw", "blank password must keep stored");
    assert_eq!(
        cfg.controller_url, "https://changed.example",
        "non-secret fields still update"
    );

    // A save with Some(nonempty) must REPLACE.
    svc.update_unifi_config(base_update(Some("second-pw".into())), actor)
        .await
        .unwrap();
    let cfg = svc.get_unifi_config().await.unwrap();
    assert_eq!(cfg.password, "second-pw", "nonempty password must replace");

    // Password is stored as ciphertext, not plaintext.
    let raw: String = sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
        .bind(unifi_keys::PASSWORD)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_ne!(raw, "second-pw", "password must be encrypted at rest");
    assert!(!raw.is_empty());
}

// ---------------------------------------------------------------------
// 5.2 — POST /test reports success/failure without persisting; a save
//        updates the store and the next gating read sees the new values.
// ---------------------------------------------------------------------

#[tokio::test]
async fn test_connection_reports_failure_without_persisting() {
    let pool = fresh_pool().await;
    let svc = settings_service(&pool);
    let actor = Uuid::nil();

    // Store known-good-looking config first.
    svc.update_unifi_config(base_update(Some("stored-pw".into())), actor)
        .await
        .unwrap();

    // Simulate the handler's test path: authenticate against an
    // unreachable controller with SUBMITTED creds, then record the result.
    let (ok, detail) = unifi::test_connection("http://127.0.0.1:1", "typed-user", "typed-pw").await;
    assert!(!ok, "unreachable controller must report failure");
    assert!(!detail.is_empty(), "failure must carry a detail message");
    svc.record_unifi_test(ok, &detail, actor).await.unwrap();

    // The submitted credentials were NOT written to the store — the stored
    // config is exactly what we saved earlier.
    let cfg = svc.get_unifi_config().await.unwrap();
    assert_eq!(cfg.controller_url, "https://controller.example");
    assert_eq!(cfg.username, "svc");
    assert_eq!(cfg.password, "stored-pw", "test must not overwrite creds");

    // Only the test-result status was recorded.
    let last_ok = svc.get_bool("unifi.last_test_ok").await.unwrap();
    assert!(!last_ok);
    let last_err = svc.get_value("unifi.last_test_error").await.unwrap();
    assert!(!last_err.is_empty());
}

#[tokio::test]
async fn save_updates_store_and_gating_reads_new_values() {
    let pool = fresh_pool().await;
    let svc = settings_service(&pool);
    let actor = Uuid::nil();

    // Initially disabled/unconfigured → gating read reflects that.
    let cfg = svc.get_unifi_config().await.unwrap();
    assert!(!cfg.enabled);
    assert!(cfg.controller_url.is_empty());

    // Save a new config; the next read (what the integration does at
    // operation time) must see the new values with no restart.
    svc.update_unifi_config(
        UpdateUnifiConfig {
            enabled: true,
            controller_url: "https://new.example".into(),
            username: "newuser".into(),
            site_id: "hq".into(),
            password: Some("newpw".into()),
        },
        actor,
    )
    .await
    .unwrap();

    let cfg = svc.get_unifi_config().await.unwrap();
    assert!(cfg.enabled);
    assert_eq!(cfg.controller_url, "https://new.example");
    assert_eq!(cfg.username, "newuser");
    assert_eq!(cfg.site_id, "hq");
    assert_eq!(cfg.password, "newpw");
}

// ---------------------------------------------------------------------
// 5.3 — .env seeds the DB once, then the DB is authoritative
// ---------------------------------------------------------------------

#[tokio::test]
async fn seed_from_env_populates_empty_db_then_env_ignored() {
    let pool = fresh_pool().await;
    let svc = settings_service(&pool);

    let env = UnifiConfig {
        enabled: true,
        controller_url: "https://env.example".into(),
        username: "env-user".into(),
        password: "env-pw".into(),
        site_id: "env-site".into(),
    };

    // DB empty + env present → seeded.
    let seeded = svc.seed_unifi_from_env(&env, SYSTEM_ACTOR).await.unwrap();
    assert!(seeded, "empty DB + env present must seed");

    let cfg = svc.get_unifi_config().await.unwrap();
    assert!(cfg.enabled);
    assert_eq!(cfg.controller_url, "https://env.example");
    assert_eq!(cfg.username, "env-user");
    assert_eq!(cfg.password, "env-pw");
    assert_eq!(cfg.site_id, "env-site");

    // DB now present → a second call with a DIFFERENT env is ignored.
    let env2 = UnifiConfig {
        enabled: true,
        controller_url: "https://DIFFERENT.example".into(),
        username: "DIFFERENT".into(),
        password: "DIFFERENT".into(),
        site_id: "DIFFERENT".into(),
    };
    let seeded2 = svc.seed_unifi_from_env(&env2, SYSTEM_ACTOR).await.unwrap();
    assert!(!seeded2, "DB present → must NOT re-seed from env");

    let cfg = svc.get_unifi_config().await.unwrap();
    assert_eq!(
        cfg.controller_url, "https://env.example",
        "portal/DB value wins over .env"
    );
}

#[tokio::test]
async fn seed_noop_when_env_absent() {
    let pool = fresh_pool().await;
    let svc = settings_service(&pool);

    let env = UnifiConfig::default(); // no controller URL
    let seeded = svc.seed_unifi_from_env(&env, SYSTEM_ACTOR).await.unwrap();
    assert!(!seeded, "no controller URL → nothing to seed");
    assert!(!svc.has_unifi_config().await, "DB stays pristine");
}

// ---------------------------------------------------------------------
// 5.4 — the generic settings page no longer carries integrations.unifi.*
// ---------------------------------------------------------------------

#[tokio::test]
async fn generic_settings_page_has_no_unifi_rows() {
    let pool = fresh_pool().await;

    // The dead row was removed by migration.
    let dead: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM app_settings WHERE key = 'integrations.unifi.enabled'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(dead, 0, "integrations.unifi.enabled row must be gone");

    // The new unifi.* rows live under the hidden `unifi` category, which
    // the generic settings page does not render.
    let svc = settings_service(&pool);
    let categories = svc.get_all_settings().await.unwrap();
    if let Some(integrations) = categories.iter().find(|c| c.name == "integrations") {
        assert!(
            !integrations
                .settings
                .iter()
                .any(|s| s.key.contains("unifi")),
            "no unifi key may appear in the rendered 'integrations' category",
        );
    }
    for s in categories.iter().flat_map(|c| &c.settings) {
        if s.key.starts_with("unifi.") {
            assert_eq!(
                s.category, "unifi",
                "unifi.* rows must be in the hidden 'unifi' category"
            );
        }
    }
}
