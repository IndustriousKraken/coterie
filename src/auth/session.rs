use chrono::{DateTime, Utc, NaiveDateTime};
use sqlx::{SqlitePool, FromRow};
use uuid::Uuid;

use crate::error::{AppError, Result};

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub member_id: Uuid,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: DateTime<Utc>,
}

#[derive(FromRow)]
struct SessionRow {
    id: String,
    member_id: String,
    token_hash: String,
    expires_at: NaiveDateTime,
    created_at: NaiveDateTime,
    last_used_at: NaiveDateTime,
}

pub struct SessionStore {
    pool: SqlitePool,
}

impl SessionStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(
        &self,
        member_id: Uuid,
        token: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<Session> {
        let id = Uuid::new_v4().to_string();
        let token_hash = hash_token(token);
        let now = Utc::now();

        let member_id_str = member_id.to_string();
        let expires_at_naive = expires_at.naive_utc();
        let now_naive = now.naive_utc();
        
        sqlx::query(
            r#"
            INSERT INTO sessions (id, member_id, token_hash, expires_at, created_at, last_used_at)
            VALUES (?, ?, ?, ?, ?, ?)
            "#
        )
        .bind(&id)
        .bind(&member_id_str)
        .bind(&token_hash)
        .bind(expires_at_naive)
        .bind(now_naive)
        .bind(now_naive)
        .execute(&self.pool)
        .await?;

        Ok(Session {
            id: id.clone(),
            member_id,
            token_hash,
            expires_at,
            created_at: now,
            last_used_at: now,
        })
    }

    pub async fn find_by_token(&self, token: &str) -> Result<Option<Session>> {
        let token_hash = hash_token(token);
        let now = Utc::now();

        let now_naive = now.naive_utc();
        
        let row = sqlx::query_as::<_, SessionRow>(
            r#"
            SELECT id, member_id, token_hash, expires_at, created_at, last_used_at
            FROM sessions
            WHERE token_hash = ? AND expires_at > ?
            "#
        )
        .bind(&token_hash)
        .bind(now_naive)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = row {
            // Update last_used_at
            sqlx::query(
                "UPDATE sessions SET last_used_at = ? WHERE id = ?"
            )
            .bind(now_naive)
            .bind(&row.id)
            .execute(&self.pool)
            .await?;

            Ok(Some(Session {
                id: row.id,
                member_id: Uuid::parse_str(&row.member_id)
                    .map_err(|e| AppError::Database(e.to_string()))?,
                token_hash: row.token_hash,
                expires_at: DateTime::from_naive_utc_and_offset(row.expires_at, Utc),
                created_at: DateTime::from_naive_utc_and_offset(row.created_at, Utc),
                last_used_at: now,
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn delete_by_token(&self, token: &str) -> Result<()> {
        let token_hash = hash_token(token);
        
        sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
            .bind(&token_hash)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn delete_by_member(&self, member_id: Uuid) -> Result<()> {
        let member_id_str = member_id.to_string();
        sqlx::query("DELETE FROM sessions WHERE member_id = ?")
            .bind(&member_id_str)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn cleanup_expired(&self) -> Result<u64> {
        let now = Utc::now();
        
        let now_naive = now.naive_utc();
        let result = sqlx::query("DELETE FROM sessions WHERE expires_at <= ?")
            .bind(now_naive)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }
}

fn hash_token(token: &str) -> String {
    use sha2::{Sha256, Digest};
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}