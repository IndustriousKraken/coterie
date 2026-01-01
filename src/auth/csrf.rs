use sha2::{Sha256, Digest};
use sqlx::SqlitePool;

use crate::error::{AppError, Result};

pub struct CsrfService {
    pool: SqlitePool,
}

impl CsrfService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Generate a new CSRF token for a session
    pub async fn generate_token(&self, session_id: &str) -> Result<String> {
        let token = generate_random_token();
        let token_hash = hash_token(&token);

        // Upsert the token (replace if exists)
        sqlx::query(
            r#"
            INSERT INTO csrf_tokens (session_id, token_hash, created_at)
            VALUES (?, ?, CURRENT_TIMESTAMP)
            ON CONFLICT(session_id) DO UPDATE SET
                token_hash = excluded.token_hash,
                created_at = CURRENT_TIMESTAMP
            "#
        )
        .bind(session_id)
        .bind(&token_hash)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(token)
    }

    /// Validate a CSRF token for a session
    pub async fn validate_token(&self, session_id: &str, token: &str) -> Result<bool> {
        let token_hash = hash_token(token);

        let result = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM csrf_tokens WHERE session_id = ? AND token_hash = ?"
        )
        .bind(session_id)
        .bind(&token_hash)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(result > 0)
    }

    /// Delete CSRF token for a session (called on logout)
    pub async fn delete_token(&self, session_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM csrf_tokens WHERE session_id = ?")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }

    /// Cleanup expired tokens (tokens for sessions that no longer exist)
    pub async fn cleanup_orphaned(&self) -> Result<u64> {
        let result = sqlx::query(
            "DELETE FROM csrf_tokens WHERE session_id NOT IN (SELECT id FROM sessions)"
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(result.rows_affected())
    }
}

fn generate_random_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_generation() {
        let token = generate_random_token();
        assert_eq!(token.len(), 64); // 32 bytes = 64 hex chars
    }

    #[test]
    fn test_token_hashing() {
        let token = "test_token";
        let hash1 = hash_token(token);
        let hash2 = hash_token(token);
        assert_eq!(hash1, hash2);
        assert_ne!(hash1, token);
    }
}
