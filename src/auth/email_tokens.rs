//! Single-use, time-limited tokens used for email verification and
//! password reset. Both table types share an identical shape and
//! lifecycle: create (hash stored, plaintext emailed), consume
//! (atomic update of `consumed_at`), and prune expired rows.
//!
//! The plaintext token only exists in the emailed URL and briefly
//! in memory during request handling — never written to disk.

use chrono::{DateTime, Duration, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use super::tokens::{generate_token, hash_token};
use crate::error::{AppError, Result};

pub struct CreatedToken {
    /// Plaintext token to include in the emailed URL. Never stored.
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

pub struct ConsumedToken {
    pub member_id: Uuid,
}

// --- Private SQL helpers (table-parameterized) ----------------------------

async fn insert_token(
    pool: &SqlitePool,
    table: &'static str,
    member_id: Uuid,
    ttl: Duration,
) -> Result<CreatedToken> {
    let token = generate_token();
    let token_hash = hash_token(&token);
    let id = Uuid::new_v4().to_string();
    let expires_at = Utc::now() + ttl;
    let expires_at_naive = expires_at.naive_utc();

    // Table name is a fixed &'static str selected by the caller,
    // never user input — no SQL-injection risk in the format!.
    let sql = format!(
        "INSERT INTO {} (id, member_id, token_hash, expires_at) VALUES (?, ?, ?, ?)",
        table
    );
    sqlx::query(&sql)
        .bind(&id)
        .bind(member_id.to_string())
        .bind(&token_hash)
        .bind(expires_at_naive)
        .execute(pool)
        .await
        .map_err(AppError::Database)?;

    Ok(CreatedToken { token, expires_at })
}

async fn consume_token(
    pool: &SqlitePool,
    table: &'static str,
    token: &str,
) -> Result<Option<ConsumedToken>> {
    let token_hash = hash_token(token);
    let now_naive = Utc::now().naive_utc();

    // Atomically flip `consumed_at`. The WHERE clause ensures we
    // only succeed on an unexpired, unconsumed token. RETURNING
    // gives us the member_id in a single round-trip.
    let sql = format!(
        "UPDATE {} \
         SET consumed_at = ? \
         WHERE token_hash = ? AND consumed_at IS NULL AND expires_at > ? \
         RETURNING member_id",
        table
    );

    let row: Option<(String,)> = sqlx::query_as(&sql)
        .bind(now_naive)
        .bind(&token_hash)
        .bind(now_naive)
        .fetch_optional(pool)
        .await
        .map_err(AppError::Database)?;

    Ok(match row {
        Some((member_id_str,)) => {
            let member_id =
                Uuid::parse_str(&member_id_str).map_err(|e| AppError::Internal(e.to_string()))?;
            Some(ConsumedToken { member_id })
        }
        None => None,
    })
}

async fn invalidate_for_member_in_table(
    pool: &SqlitePool,
    table: &'static str,
    member_id: Uuid,
) -> Result<()> {
    let sql = format!(
        "UPDATE {} SET consumed_at = CURRENT_TIMESTAMP \
         WHERE member_id = ? AND consumed_at IS NULL",
        table
    );
    sqlx::query(&sql)
        .bind(member_id.to_string())
        .execute(pool)
        .await
        .map_err(AppError::Database)?;
    Ok(())
}

async fn cleanup_expired_in_table(pool: &SqlitePool, table: &'static str) -> Result<u64> {
    let sql = format!("DELETE FROM {} WHERE expires_at <= ?", table);
    let result = sqlx::query(&sql)
        .bind(Utc::now().naive_utc())
        .execute(pool)
        .await
        .map_err(AppError::Database)?;
    Ok(result.rows_affected())
}

// --- Public free functions: verification tokens --------------------------

pub async fn create_verification_token(
    pool: &SqlitePool,
    member_id: Uuid,
    ttl: Duration,
) -> Result<CreatedToken> {
    insert_token(pool, "email_verification_tokens", member_id, ttl).await
}

pub async fn consume_verification_token(
    pool: &SqlitePool,
    token: &str,
) -> Result<Option<ConsumedToken>> {
    consume_token(pool, "email_verification_tokens", token).await
}

pub async fn invalidate_verification_tokens_for_member(
    pool: &SqlitePool,
    member_id: Uuid,
) -> Result<()> {
    invalidate_for_member_in_table(pool, "email_verification_tokens", member_id).await
}

#[allow(dead_code)]
pub async fn cleanup_expired_verification_tokens(pool: &SqlitePool) -> Result<u64> {
    cleanup_expired_in_table(pool, "email_verification_tokens").await
}

// --- Public free functions: password-reset tokens ------------------------

pub async fn create_password_reset_token(
    pool: &SqlitePool,
    member_id: Uuid,
    ttl: Duration,
) -> Result<CreatedToken> {
    insert_token(pool, "password_reset_tokens", member_id, ttl).await
}

pub async fn consume_password_reset_token(
    pool: &SqlitePool,
    token: &str,
) -> Result<Option<ConsumedToken>> {
    consume_token(pool, "password_reset_tokens", token).await
}

pub async fn invalidate_password_reset_tokens_for_member(
    pool: &SqlitePool,
    member_id: Uuid,
) -> Result<()> {
    invalidate_for_member_in_table(pool, "password_reset_tokens", member_id).await
}

#[allow(dead_code)]
pub async fn cleanup_expired_password_reset_tokens(pool: &SqlitePool) -> Result<u64> {
    cleanup_expired_in_table(pool, "password_reset_tokens").await
}
