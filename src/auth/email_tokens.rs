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

use crate::error::{AppError, Result};

pub struct EmailTokenService {
    pool: SqlitePool,
    /// One of the token tables: "email_verification_tokens" or
    /// "password_reset_tokens". Hard-coded; never user input.
    table: &'static str,
}

pub struct CreatedToken {
    /// Plaintext token to include in the emailed URL. Never stored.
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

pub struct ConsumedToken {
    pub member_id: Uuid,
}

impl EmailTokenService {
    pub fn verification(pool: SqlitePool) -> Self {
        Self { pool, table: "email_verification_tokens" }
    }

    pub fn password_reset(pool: SqlitePool) -> Self {
        Self { pool, table: "password_reset_tokens" }
    }

    /// Generate a new token, store its hash, and return the plaintext
    /// token (to be emailed) along with the expiry timestamp.
    pub async fn create(&self, member_id: Uuid, ttl: Duration) -> Result<CreatedToken> {
        let token = generate_token();
        let token_hash = hash_token(&token);
        let id = Uuid::new_v4().to_string();
        let expires_at = Utc::now() + ttl;
        let expires_at_naive = expires_at.naive_utc();

        // Table name is a fixed &'static str selected by constructor,
        // never user input — no SQL-injection risk in the format!.
        let sql = format!(
            "INSERT INTO {} (id, member_id, token_hash, expires_at) VALUES (?, ?, ?, ?)",
            self.table
        );
        sqlx::query(&sql)
            .bind(&id)
            .bind(member_id.to_string())
            .bind(&token_hash)
            .bind(expires_at_naive)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(CreatedToken { token, expires_at })
    }

    /// Validate and atomically consume a token. Returns the member_id
    /// if the token was valid (exists, not expired, not already
    /// consumed). After this call the token is permanently unusable.
    pub async fn consume(&self, token: &str) -> Result<Option<ConsumedToken>> {
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
            self.table
        );

        let row: Option<(String,)> = sqlx::query_as(&sql)
            .bind(now_naive)
            .bind(&token_hash)
            .bind(now_naive)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(match row {
            Some((member_id_str,)) => {
                let member_id = Uuid::parse_str(&member_id_str)
                    .map_err(|e| AppError::Database(e.to_string()))?;
                Some(ConsumedToken { member_id })
            }
            None => None,
        })
    }

    /// Invalidate all outstanding tokens for a member. Useful after
    /// successful verification/reset so other in-flight tokens can't
    /// be used.
    pub async fn invalidate_for_member(&self, member_id: Uuid) -> Result<()> {
        let sql = format!(
            "UPDATE {} SET consumed_at = CURRENT_TIMESTAMP \
             WHERE member_id = ? AND consumed_at IS NULL",
            self.table
        );
        sqlx::query(&sql)
            .bind(member_id.to_string())
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    /// Delete expired tokens. Intended to be called periodically.
    #[allow(dead_code)]
    pub async fn cleanup_expired(&self) -> Result<u64> {
        let sql = format!("DELETE FROM {} WHERE expires_at <= ?", self.table);
        let result = sqlx::query(&sql)
            .bind(Utc::now().naive_utc())
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(result.rows_affected())
    }
}

fn generate_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn hash_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}
