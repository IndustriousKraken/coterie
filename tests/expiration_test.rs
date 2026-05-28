//! Integration tests for `Expiration::check_expired_members`, the
//! daily sweep that flips Active members past dues + grace period to
//! `Expired`, kills their live sessions, and dispatches
//! `IntegrationEvent::MemberExpired`.
//!
//! Hits a real in-memory SQLite + migrations and drives `Expiration`
//! directly (no `BillingService` facade) since the sweep is
//! self-contained. A `RecordingIntegration` (same shape as
//! `tests/auto_renew_alert_test.rs`) captures every dispatched
//! `IntegrationEvent` so the tests can assert which events fired.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use coterie::{
    auth::SecretCrypto,
    domain::CreateMemberRequest,
    error::Result as CoterieResult,
    integrations::{Integration, IntegrationEvent, IntegrationManager},
    repository::{MemberRepository, SqliteMemberRepository},
    service::{billing_service::expiration::Expiration, settings_service::SettingsService},
};
use sqlx::SqlitePool;
use uuid::Uuid;

mod common;
use common::fresh_pool;

// ---------------------------------------------------------------------
// RecordingIntegration — test-only Integration impl that captures every
// dispatched IntegrationEvent into a shared Vec the test can inspect.
// Co-located to keep the test file self-contained.
// ---------------------------------------------------------------------

struct RecordingIntegration {
    events: Arc<Mutex<Vec<IntegrationEvent>>>,
}

#[async_trait]
impl Integration for RecordingIntegration {
    fn name(&self) -> &str {
        "test-recording"
    }
    fn is_enabled(&self) -> bool {
        true
    }
    async fn health_check(&self) -> CoterieResult<()> {
        Ok(())
    }
    async fn handle_event(&self, event: &IntegrationEvent) -> CoterieResult<()> {
        self.events.lock().unwrap().push(event.clone());
        Ok(())
    }
}

fn member_expired_ids(events: &Arc<Mutex<Vec<IntegrationEvent>>>) -> Vec<Uuid> {
    events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            IntegrationEvent::MemberExpired(m) => Some(m.id),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------

struct Harness {
    pool: SqlitePool,
    expiration: Expiration,
    recorded_events: Arc<Mutex<Vec<IntegrationEvent>>>,
}

async fn build_harness() -> Harness {
    let pool = fresh_pool().await;
    let crypto = Arc::new(SecretCrypto::new("test-secret-please-ignore"));
    let settings_service = Arc::new(SettingsService::new(pool.clone(), crypto));
    let member_repo: Arc<dyn MemberRepository> =
        Arc::new(SqliteMemberRepository::new(pool.clone()));

    let integration_manager = Arc::new(IntegrationManager::new());
    let recorded_events: Arc<Mutex<Vec<IntegrationEvent>>> = Arc::new(Mutex::new(Vec::new()));
    integration_manager
        .register(Arc::new(RecordingIntegration {
            events: recorded_events.clone(),
        }))
        .await;

    let expiration = Expiration::new(
        member_repo,
        settings_service,
        integration_manager,
        pool.clone(),
    );

    Harness {
        pool,
        expiration,
        recorded_events,
    }
}

// ---------------------------------------------------------------------
// Seed helpers
// ---------------------------------------------------------------------

/// Insert an Active member with `dues_paid_until` set to `now + days`
/// (use a negative number to put them in the past). `bypass_dues` is
/// set to the requested value. `SqliteMemberRepository::create` writes
/// the row as Pending — we then UPDATE it into the state the sweep
/// expects.
async fn seed_active_member(pool: &SqlitePool, days_offset: i64, bypass_dues: bool) -> Uuid {
    let repo = SqliteMemberRepository::new(pool.clone());
    let member = repo
        .create(CreateMemberRequest {
            email: format!("m-{}@example.com", Uuid::new_v4()),
            username: format!("u_{}", Uuid::new_v4().simple()),
            full_name: "Test Member".to_string(),
            password: "p4ssword_long_enough".to_string(),
            membership_type_id: None,
            ..Default::default()
        })
        .await
        .expect("create member");

    let dues_until = Utc::now() + Duration::days(days_offset);

    sqlx::query(
        "UPDATE members \
         SET status = 'Active', dues_paid_until = ?, bypass_dues = ? \
         WHERE id = ?",
    )
    .bind(dues_until)
    .bind(bypass_dues as i32)
    .bind(member.id.to_string())
    .execute(pool)
    .await
    .expect("stamp Active + dues + bypass");

    member.id
}

async fn member_status(pool: &SqlitePool, id: Uuid) -> String {
    sqlx::query_scalar::<_, String>("SELECT status FROM members WHERE id = ?")
        .bind(id.to_string())
        .fetch_one(pool)
        .await
        .expect("query member status")
}

/// Overwrite the seeded `membership.grace_period_days` row with the
/// requested value. Migration 001 inserts a default of `30`; tests that
/// care about a specific grace period stamp it here.
async fn set_grace_period(pool: &SqlitePool, days: i64) {
    sqlx::query("UPDATE app_settings SET value = ? WHERE key = 'membership.grace_period_days'")
        .bind(days.to_string())
        .execute(pool)
        .await
        .expect("set grace period");
}

/// Remove the seeded `membership.grace_period_days` row entirely so
/// `SettingsService::get_number` returns Err and the sweep's
/// `.unwrap_or(3)` fallback kicks in.
async fn delete_grace_period_setting(pool: &SqlitePool) {
    sqlx::query("DELETE FROM app_settings WHERE key = 'membership.grace_period_days'")
        .execute(pool)
        .await
        .expect("delete grace period setting");
}

/// Insert a live `sessions` row for `member_id` whose `expires_at` is
/// well in the future. Returns the session id.
async fn seed_session(pool: &SqlitePool, member_id: Uuid) -> String {
    let session_id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let expires_at = now + Duration::hours(24);
    sqlx::query(
        "INSERT INTO sessions (id, member_id, token_hash, expires_at, created_at, last_used_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&session_id)
    .bind(member_id.to_string())
    .bind(format!("hash_{}", session_id))
    .bind(expires_at.naive_utc())
    .bind(now.naive_utc())
    .bind(now.naive_utc())
    .execute(pool)
    .await
    .expect("insert session");
    session_id
}

async fn session_count_for(pool: &SqlitePool, member_id: Uuid) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sessions WHERE member_id = ?")
        .bind(member_id.to_string())
        .fetch_one(pool)
        .await
        .expect("count sessions")
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[tokio::test]
async fn expires_active_member_past_grace_period() {
    let h = build_harness().await;
    let member_id = seed_active_member(&h.pool, -10, false).await;
    set_grace_period(&h.pool, 3).await;

    let count = h
        .expiration
        .check_expired_members()
        .await
        .expect("sweep ok");

    assert_eq!(count, 1, "expected exactly one member to flip Expired");
    assert_eq!(member_status(&h.pool, member_id).await, "Expired");

    let expired_ids = member_expired_ids(&h.recorded_events);
    assert_eq!(
        expired_ids,
        vec![member_id],
        "expected exactly one MemberExpired event for the seeded member"
    );
}

#[tokio::test]
async fn does_not_expire_member_within_grace_period() {
    let h = build_harness().await;
    let member_id = seed_active_member(&h.pool, -1, false).await;
    set_grace_period(&h.pool, 3).await;

    let count = h
        .expiration
        .check_expired_members()
        .await
        .expect("sweep ok");

    assert_eq!(count, 0, "member 1 day past dues with 3-day grace must not flip");
    assert_eq!(member_status(&h.pool, member_id).await, "Active");
    assert!(
        member_expired_ids(&h.recorded_events).is_empty(),
        "no MemberExpired event should be dispatched"
    );
}

#[tokio::test]
async fn does_not_expire_bypass_dues_member() {
    let h = build_harness().await;
    let member_id = seed_active_member(&h.pool, -999, true).await;
    set_grace_period(&h.pool, 3).await;

    let count = h
        .expiration
        .check_expired_members()
        .await
        .expect("sweep ok");

    assert_eq!(count, 0, "bypass_dues members must never be swept");
    assert_eq!(member_status(&h.pool, member_id).await, "Active");
    assert!(
        member_expired_ids(&h.recorded_events).is_empty(),
        "no MemberExpired event should fire for a bypass_dues member"
    );
}

#[tokio::test]
async fn expiration_invalidates_live_sessions() {
    let h = build_harness().await;
    let member_id = seed_active_member(&h.pool, -10, false).await;
    set_grace_period(&h.pool, 3).await;
    seed_session(&h.pool, member_id).await;

    assert_eq!(
        session_count_for(&h.pool, member_id).await,
        1,
        "precondition: seeded session row should be present"
    );

    let count = h
        .expiration
        .check_expired_members()
        .await
        .expect("sweep ok");

    assert_eq!(count, 1);
    assert_eq!(
        session_count_for(&h.pool, member_id).await,
        0,
        "expired member's session must be deleted"
    );
    assert_eq!(member_status(&h.pool, member_id).await, "Expired");
}

#[tokio::test]
async fn expiration_uses_default_grace_when_setting_unset() {
    let h = build_harness().await;
    let member_id = seed_active_member(&h.pool, -5, false).await;
    delete_grace_period_setting(&h.pool).await;

    let count = h
        .expiration
        .check_expired_members()
        .await
        .expect("sweep ok");

    assert_eq!(
        count, 1,
        "default grace of 3 days must apply when setting is unset (5 > 3)"
    );
    assert_eq!(member_status(&h.pool, member_id).await, "Expired");
}
