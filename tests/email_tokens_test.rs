//! Integration tests for the email-token free functions. They are the
//! security-critical backend for email-verification and password-reset
//! tokens; these tests lock in the single-use, expiry, cross-purpose,
//! invalidate, and cleanup invariants spelled out in
//! `openspec/specs/email-tokens/spec.md`.
//!
//! Hits a real in-memory SQLite + migrations (same harness shape as
//! `totp_test.rs`).
//!
//! Run with: cargo test --test email_tokens_test

use chrono::{Duration, Utc};
use coterie::{
    auth::email_tokens::{
        cleanup_expired_password_reset_tokens, consume_password_reset_token,
        consume_verification_token, create_password_reset_token, create_verification_token,
        invalidate_password_reset_tokens_for_member,
    },
    domain::CreateMemberRequest,
    repository::{MemberRepository, SqliteMemberRepository},
};
use sha2::{Digest, Sha256};
use sqlx::{Executor, SqlitePool};
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

async fn make_member(pool: &SqlitePool) -> Uuid {
    let repo = SqliteMemberRepository::new(pool.clone());
    let member = repo
        .create(CreateMemberRequest {
            email: format!("u-{}@example.com", Uuid::new_v4()),
            username: format!("user_{}", Uuid::new_v4().simple()),
            full_name: "Test User".to_string(),
            password: "p4ssword_long_enough".to_string(),
            membership_type_id: None,
        })
        .await
        .expect("create member");
    member.id
}

fn sha256_hex(plaintext: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(plaintext.as_bytes());
    hex::encode(hasher.finalize())
}

// --------------------------------------------------------------------
// 1. Single-use and expiry on consume
// --------------------------------------------------------------------

#[tokio::test]
async fn consume_redeems_exactly_once() {
    let pool = fresh_pool().await;
    let member_id = make_member(&pool).await;

    let created = create_password_reset_token(&pool, member_id, Duration::hours(1))
        .await
        .expect("create");

    let first = consume_password_reset_token(&pool, &created.token)
        .await
        .expect("consume #1");
    let first = first.expect("first consume returns Some");
    assert_eq!(first.member_id, member_id);

    let second = consume_password_reset_token(&pool, &created.token)
        .await
        .expect("consume #2");
    assert!(second.is_none(), "second consume of same token must return None");
}

#[tokio::test]
async fn consume_rejects_expired_token() {
    let pool = fresh_pool().await;
    let member_id = make_member(&pool).await;

    // Insert a row directly with expires_at in the past so we don't
    // wait for real time to pass. We pick our own plaintext so we can
    // pass it to consume() afterwards.
    let plaintext = "expired-plaintext-token-for-test";
    let token_hash = sha256_hex(plaintext);
    let past = (Utc::now() - Duration::hours(1)).naive_utc();
    sqlx::query(
        "INSERT INTO password_reset_tokens (id, member_id, token_hash, expires_at) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(member_id.to_string())
    .bind(&token_hash)
    .bind(past)
    .execute(&pool)
    .await
    .expect("seed expired row");

    let consumed = consume_password_reset_token(&pool, plaintext)
        .await
        .expect("consume");
    assert!(consumed.is_none(), "expired token must not consume");

    // The gated UPDATE must have done nothing — consumed_at stays NULL.
    let consumed_at: Option<chrono::NaiveDateTime> = sqlx::query_scalar(
        "SELECT consumed_at FROM password_reset_tokens WHERE token_hash = ?",
    )
    .bind(&token_hash)
    .fetch_one(&pool)
    .await
    .expect("read consumed_at");
    assert!(consumed_at.is_none(), "expired token's consumed_at must remain NULL");
}

#[tokio::test]
async fn consume_rejects_unknown_token() {
    let pool = fresh_pool().await;

    let result = consume_password_reset_token(&pool, "not-a-real-token")
        .await
        .expect("consume");
    assert!(result.is_none(), "unknown plaintext must return Ok(None)");
}

// --------------------------------------------------------------------
// 2. Cross-purpose isolation: separate tables
// --------------------------------------------------------------------

#[tokio::test]
async fn password_reset_token_does_not_consume_at_verification_service() {
    let pool = fresh_pool().await;
    let member_id = make_member(&pool).await;

    let created = create_password_reset_token(&pool, member_id, Duration::hours(1))
        .await
        .expect("create");

    let crossed = consume_verification_token(&pool, &created.token)
        .await
        .expect("consume");
    assert!(
        crossed.is_none(),
        "reset token must not redeem at verification function (separate table)"
    );

    // Sanity: original token still works at the correct function.
    assert!(consume_password_reset_token(&pool, &created.token)
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn verification_token_does_not_consume_at_password_reset_service() {
    let pool = fresh_pool().await;
    let member_id = make_member(&pool).await;

    let created = create_verification_token(&pool, member_id, Duration::hours(1))
        .await
        .expect("create");

    let crossed = consume_password_reset_token(&pool, &created.token)
        .await
        .expect("consume");
    assert!(
        crossed.is_none(),
        "verification token must not redeem at password-reset function (separate table)"
    );

    assert!(consume_verification_token(&pool, &created.token)
        .await
        .unwrap()
        .is_some());
}

// --------------------------------------------------------------------
// 3. invalidate_for_member and cleanup_expired
// --------------------------------------------------------------------

#[tokio::test]
async fn invalidate_for_member_marks_outstanding_consumed() {
    let pool = fresh_pool().await;
    let member_id = make_member(&pool).await;

    let a = create_password_reset_token(&pool, member_id, Duration::hours(1))
        .await
        .expect("create a");
    let b = create_password_reset_token(&pool, member_id, Duration::hours(1))
        .await
        .expect("create b");

    invalidate_password_reset_tokens_for_member(&pool, member_id)
        .await
        .expect("invalidate");

    assert!(
        consume_password_reset_token(&pool, &a.token).await.unwrap().is_none(),
        "token a must be unusable after invalidate_for_member"
    );
    assert!(
        consume_password_reset_token(&pool, &b.token).await.unwrap().is_none(),
        "token b must be unusable after invalidate_for_member"
    );
}

#[tokio::test]
async fn cleanup_expired_deletes_only_expired_rows() {
    let pool = fresh_pool().await;
    let member_id = make_member(&pool).await;

    // One fresh token via the function.
    let fresh = create_password_reset_token(&pool, member_id, Duration::hours(1))
        .await
        .expect("create fresh");

    // One already-expired row inserted directly.
    let expired_plaintext = "expired-row-for-cleanup-test";
    let expired_hash = sha256_hex(expired_plaintext);
    let past = (Utc::now() - Duration::hours(1)).naive_utc();
    sqlx::query(
        "INSERT INTO password_reset_tokens (id, member_id, token_hash, expires_at) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(member_id.to_string())
    .bind(&expired_hash)
    .bind(past)
    .execute(&pool)
    .await
    .expect("seed expired row");

    let deleted = cleanup_expired_password_reset_tokens(&pool)
        .await
        .expect("cleanup");
    assert_eq!(deleted, 1, "cleanup_expired must delete exactly the expired row");

    // The fresh token still redeems — cleanup did not touch it.
    let consumed = consume_password_reset_token(&pool, &fresh.token)
        .await
        .expect("consume fresh");
    assert!(consumed.is_some(), "unexpired token must still redeem after cleanup");
}

// --------------------------------------------------------------------
// 4. Storage invariant: stored hash equals SHA-256(plaintext)
// --------------------------------------------------------------------

#[tokio::test]
async fn created_token_is_sha256_of_plaintext() {
    let pool = fresh_pool().await;
    let member_id = make_member(&pool).await;

    let created = create_password_reset_token(&pool, member_id, Duration::hours(1))
        .await
        .expect("create");

    let stored_hash: String = sqlx::query_scalar(
        "SELECT token_hash FROM password_reset_tokens WHERE member_id = ?",
    )
    .bind(member_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("read token_hash");

    let expected = sha256_hex(&created.token);
    assert_eq!(
        stored_hash, expected,
        "token_hash must be hex(SHA-256(plaintext)) with no other transformation"
    );
}
