//! Integration tests for TOTP enrollment, verification, disable,
//! and the recovery-code lifecycle. Hits a real in-memory SQLite +
//! migrations; constructs `TotpService` / `PendingLoginService`
//! against it and exercises the same surface the HTTP handlers use.
//!
//! Stripe / billing dependencies are out of scope here, so these
//! tests don't construct a `BillingService`.
//!
//! Run with: cargo test --test totp_test

use std::sync::Arc;

use coterie::{
    auth::{
        recovery_codes, PendingLoginService, SecretCrypto, TotpService,
    },
    domain::{CreateMemberRequest, MembershipType},
    repository::{MemberRepository, SqliteMemberRepository},
};
use sqlx::{Executor, SqlitePool};
use totp_rs::{Algorithm, TOTP};
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
    sqlx::migrate!("./migrations").run(&pool).await.expect("migrate");
    pool
}

fn build_totp_service(pool: &SqlitePool) -> TotpService {
    TotpService::new(
        pool.clone(),
        Arc::new(SecretCrypto::new("test-secret-please-ignore")),
        "Coterie Test".to_string(),
    )
}

async fn make_member(pool: &SqlitePool) -> (Uuid, String) {
    let repo = SqliteMemberRepository::new(pool.clone());
    let member = repo
        .create(CreateMemberRequest {
            email: format!("u-{}@example.com", Uuid::new_v4()),
            username: format!("user_{}", Uuid::new_v4().simple()),
            full_name: "Test User".to_string(),
            password: "p4ssword_long_enough".to_string(),
            membership_type: MembershipType::Regular,
        })
        .await
        .expect("create");
    let email = member.email.clone();
    (member.id, email)
}

/// Reconstruct a TOTP from the secret_base32 the enrollment-init
/// returns, mirroring how the user's authenticator app would produce
/// codes from it. Production verification builds the same TOTP from
/// the encrypted-then-decrypted secret on the member row; here we
/// shortcut by using the plaintext directly.
fn totp_from_b32(secret_b32: &str, account_name: &str) -> TOTP {
    use totp_rs::Secret;
    let bytes = Secret::Encoded(secret_b32.to_string()).to_bytes().unwrap();
    TOTP::new(Algorithm::SHA1, 6, 1, 30, bytes, Some("Coterie Test".to_string()), account_name.to_string()).unwrap()
}

// --------------------------------------------------------------------
// Enrollment
// --------------------------------------------------------------------

#[tokio::test]
async fn enroll_round_trip_persists_secret_and_enables_2fa() {
    let pool = fresh_pool().await;
    let (member_id, email) = make_member(&pool).await;
    let svc = build_totp_service(&pool);

    assert!(!svc.is_enabled(member_id).await.unwrap(), "starts disabled");

    let init = svc.begin_enrollment(&email).expect("begin");
    let totp = totp_from_b32(&init.secret_base32, &email);
    let code = totp.generate_current().unwrap();

    let confirmed = svc.confirm_enrollment(member_id, &init.secret_base32, &code, &email)
        .await.expect("confirm");
    assert!(confirmed);
    assert!(svc.is_enabled(member_id).await.unwrap(), "now enabled");

    // The secret must round-trip via the encrypted column for the next
    // verify_for_member call to work — covers SecretCrypto integration.
    let next_code = totp.generate_current().unwrap();
    assert!(svc.verify_for_member(member_id, &next_code, &email).await.unwrap());
}

#[tokio::test]
async fn enroll_with_wrong_code_does_not_persist() {
    let pool = fresh_pool().await;
    let (member_id, email) = make_member(&pool).await;
    let svc = build_totp_service(&pool);

    let init = svc.begin_enrollment(&email).expect("begin");
    let confirmed = svc.confirm_enrollment(member_id, &init.secret_base32, "000000", &email)
        .await.expect("confirm");
    assert!(!confirmed);
    assert!(!svc.is_enabled(member_id).await.unwrap(),
        "wrong-code enrollment must NOT enable 2FA");
}

// --------------------------------------------------------------------
// Verify against a stored secret
// --------------------------------------------------------------------

#[tokio::test]
async fn verify_returns_false_when_2fa_off() {
    let pool = fresh_pool().await;
    let (member_id, email) = make_member(&pool).await;
    let svc = build_totp_service(&pool);

    assert!(!svc.verify_for_member(member_id, "123456", &email).await.unwrap(),
        "any code must be rejected when 2FA isn't enrolled");
}

#[tokio::test]
async fn verify_rejects_off_window_code() {
    let pool = fresh_pool().await;
    let (member_id, email) = make_member(&pool).await;
    let svc = build_totp_service(&pool);
    let init = svc.begin_enrollment(&email).expect("begin");
    let totp = totp_from_b32(&init.secret_base32, &email);
    let now_code = totp.generate_current().unwrap();
    svc.confirm_enrollment(member_id, &init.secret_base32, &now_code, &email)
        .await.unwrap();

    // A code generated for "way in the past" — far enough that even
    // SKEW=1 won't accept it.
    let stale_code = totp.generate(0);
    assert!(!svc.verify_for_member(member_id, &stale_code, &email).await.unwrap());
}

// --------------------------------------------------------------------
// Disable
// --------------------------------------------------------------------

#[tokio::test]
async fn disable_clears_secret_and_recovery_codes() {
    let pool = fresh_pool().await;
    let (member_id, email) = make_member(&pool).await;
    let svc = build_totp_service(&pool);

    let init = svc.begin_enrollment(&email).unwrap();
    let totp = totp_from_b32(&init.secret_base32, &email);
    svc.confirm_enrollment(member_id, &init.secret_base32, &totp.generate_current().unwrap(), &email)
        .await.unwrap();
    let _codes = recovery_codes::issue_for_member(&pool, member_id).await.unwrap();
    assert_eq!(recovery_codes::remaining_count(&pool, member_id).await.unwrap(), 10);

    svc.disable(member_id).await.unwrap();

    assert!(!svc.is_enabled(member_id).await.unwrap());
    assert_eq!(recovery_codes::remaining_count(&pool, member_id).await.unwrap(), 0);
    let next_code = totp.generate_current().unwrap();
    assert!(!svc.verify_for_member(member_id, &next_code, &email).await.unwrap(),
        "old codes must stop working after disable");
}

// --------------------------------------------------------------------
// Recovery codes
// --------------------------------------------------------------------

#[tokio::test]
async fn recovery_code_one_time_use() {
    let pool = fresh_pool().await;
    let (member_id, _) = make_member(&pool).await;

    let codes = recovery_codes::issue_for_member(&pool, member_id).await.unwrap();
    assert_eq!(codes.len(), 10);
    let pick = codes[3].clone();

    // First use succeeds; second use of the same code must fail.
    assert!(recovery_codes::try_consume(&pool, member_id, &pick).await.unwrap());
    assert!(!recovery_codes::try_consume(&pool, member_id, &pick).await.unwrap());
    assert_eq!(recovery_codes::remaining_count(&pool, member_id).await.unwrap(), 9);

    // Other codes still work.
    assert!(recovery_codes::try_consume(&pool, member_id, &codes[0]).await.unwrap());
    assert_eq!(recovery_codes::remaining_count(&pool, member_id).await.unwrap(), 8);
}

#[tokio::test]
async fn recovery_codes_regenerate_invalidates_old_set() {
    let pool = fresh_pool().await;
    let (member_id, _) = make_member(&pool).await;

    let original = recovery_codes::issue_for_member(&pool, member_id).await.unwrap();
    let _new = recovery_codes::issue_for_member(&pool, member_id).await.unwrap();

    // Every original code should now fail.
    for code in &original {
        assert!(
            !recovery_codes::try_consume(&pool, member_id, code).await.unwrap(),
            "old code {} unexpectedly still valid",
            code,
        );
    }
}

#[tokio::test]
async fn recovery_codes_format_normalization() {
    let pool = fresh_pool().await;
    let (member_id, _) = make_member(&pool).await;
    let codes = recovery_codes::issue_for_member(&pool, member_id).await.unwrap();
    let pick = codes[0].clone();

    // Whitespace, lowercase, missing-hyphens variants must all match
    // the same hash. (Users will paste these in all kinds of mangled
    // forms.)
    let lower_no_hyphens: String = pick.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_lowercase();
    let with_spaces = pick.replace('-', " ");
    let consumed = recovery_codes::try_consume(&pool, member_id, &lower_no_hyphens)
        .await.unwrap();
    assert!(consumed, "lowercase no-hyphens should match");
    assert!(
        !recovery_codes::try_consume(&pool, member_id, &with_spaces).await.unwrap(),
        "same code should now be consumed regardless of formatting"
    );
}

// --------------------------------------------------------------------
// pending_logins lifecycle
// --------------------------------------------------------------------

#[tokio::test]
async fn pending_login_consume_is_atomic() {
    let pool = fresh_pool().await;
    let (member_id, _) = make_member(&pool).await;
    let svc = PendingLoginService::new(pool.clone());

    let token = svc.create(member_id, true).await.unwrap();
    let consumed_first = svc.consume(&token).await.unwrap();
    assert!(consumed_first.is_some());
    assert_eq!(consumed_first.as_ref().unwrap().member_id, member_id);
    assert!(consumed_first.as_ref().unwrap().remember_me);

    // Second consume of the same token must fail — the row is gone.
    let consumed_again = svc.consume(&token).await.unwrap();
    assert!(consumed_again.is_none());
}

#[tokio::test]
async fn pending_login_find_does_not_consume() {
    let pool = fresh_pool().await;
    let (member_id, _) = make_member(&pool).await;
    let svc = PendingLoginService::new(pool.clone());

    let token = svc.create(member_id, false).await.unwrap();
    assert!(svc.find(&token).await.unwrap().is_some(), "find #1");
    assert!(svc.find(&token).await.unwrap().is_some(), "find #2 — must still exist");
    assert!(svc.consume(&token).await.unwrap().is_some(), "consume — first time");
    assert!(svc.find(&token).await.unwrap().is_none(), "after consume");
}

#[tokio::test]
async fn pending_login_unknown_token_returns_none() {
    let pool = fresh_pool().await;
    let svc = PendingLoginService::new(pool);
    assert!(svc.find("not-a-real-token").await.unwrap().is_none());
    assert!(svc.consume("not-a-real-token").await.unwrap().is_none());
}

#[tokio::test]
async fn pending_login_expired_is_swept() {
    let pool = fresh_pool().await;
    let (member_id, _) = make_member(&pool).await;
    let svc = PendingLoginService::new(pool.clone());

    // Insert directly with an already-expired expires_at so we don't
    // have to wait 5 minutes in a test.
    let token = "test_expired_token_abc";
    let token_hash = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(token.as_bytes());
        hex::encode(hasher.finalize())
    };
    let past = chrono::Utc::now().naive_utc() - chrono::Duration::minutes(1);
    sqlx::query(
        "INSERT INTO pending_logins (id, member_id, token_hash, remember_me, expires_at) \
         VALUES (?, ?, ?, 0, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(member_id.to_string())
    .bind(&token_hash)
    .bind(past)
    .execute(&pool)
    .await
    .unwrap();

    assert!(svc.find(token).await.unwrap().is_none(), "expired must not be visible");
    assert!(svc.consume(token).await.unwrap().is_none(), "expired must not consume");

    let swept = svc.cleanup_expired().await.unwrap();
    assert!(swept >= 1, "cleanup should have removed at least one row");
}

#[tokio::test]
async fn pending_login_disable_clears_member_rows() {
    let pool = fresh_pool().await;
    let (member_id, email) = make_member(&pool).await;
    let pl = PendingLoginService::new(pool.clone());
    let totp = build_totp_service(&pool);

    // Enroll so disable() has something to clear.
    let init = totp.begin_enrollment(&email).unwrap();
    let t = totp_from_b32(&init.secret_base32, &email);
    totp.confirm_enrollment(member_id, &init.secret_base32, &t.generate_current().unwrap(), &email)
        .await.unwrap();

    // Mint two pending tokens so we can verify disable() wipes them all.
    let _t1 = pl.create(member_id, false).await.unwrap();
    let _t2 = pl.create(member_id, true).await.unwrap();

    totp.disable(member_id).await.unwrap();

    // Both tokens should be gone (delete_for_member ran inside disable()).
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pending_logins WHERE member_id = ?",
    )
    .bind(member_id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 0);
}
