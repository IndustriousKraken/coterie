//! Short-lived intermediate state between password verification and
//! TOTP verification. A successful POST /login for a member with 2FA
//! enabled mints a `pending_login`, sets the corresponding cookie,
//! and redirects to /login/totp. POST /login/totp consumes the row
//! and creates a real session.
//!
//! Distinct from `sessions` so the auth middleware can stay simple:
//! `require_auth` reads only `sessions` — a half-finished login can
//! never accidentally satisfy the guard. The two cookies have
//! different names (`session` vs `pending_login`).
//!
//! Lifetime is 5 minutes — long enough for a user to fish out their
//! phone, short enough that an attacker who steals a cookie has a
//! very small window to use it.

use chrono::{DateTime, Duration, Utc};
use cookie::{Cookie, SameSite};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::error::{AppError, Result};

pub const COOKIE_NAME: &str = "pending_login";

/// 5 minutes. Exposed to the wider auth module if it ever needs to
/// reflect the value into the UI ("session expires in N seconds").
pub const TTL: Duration = Duration::minutes(5);

pub struct PendingLogin {
    pub member_id: Uuid,
    pub remember_me: bool,
    pub expires_at: DateTime<Utc>,
}

pub struct PendingLoginService {
    pool: SqlitePool,
}

impl PendingLoginService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Mint a fresh pending-login row and return the plaintext token
    /// that the caller should set on the cookie. The cookie is
    /// deliberately separate from `session` — see module docs.
    pub async fn create(&self, member_id: Uuid, remember_me: bool) -> Result<String> {
        let token = generate_token();
        let token_hash = hash_token(&token);
        let id = Uuid::new_v4().to_string();
        let expires_at = (Utc::now() + TTL).naive_utc();

        sqlx::query(
            "INSERT INTO pending_logins (id, member_id, token_hash, remember_me, expires_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(member_id.to_string())
        .bind(&token_hash)
        .bind(if remember_me { 1i32 } else { 0 })
        .bind(expires_at)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(token)
    }

    /// Look up an unexpired pending-login by its plaintext token. Does
    /// NOT delete the row — that happens via `consume` after the TOTP
    /// step succeeds. Returning the row without consuming it lets the
    /// /login/totp page render the form even after a failed code
    /// submission, without forcing the user back to /login.
    pub async fn find(&self, token: &str) -> Result<Option<PendingLogin>> {
        let token_hash = hash_token(token);
        let now = Utc::now().naive_utc();

        let row: Option<(String, i32, DateTime<Utc>)> = sqlx::query_as(
            "SELECT member_id, remember_me, expires_at \
             FROM pending_logins \
             WHERE token_hash = ? AND expires_at > ? \
             LIMIT 1",
        )
        .bind(&token_hash)
        .bind(now)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(match row {
            Some((member_id_str, remember_me, expires_at)) => {
                let member_id = Uuid::parse_str(&member_id_str)
                    .map_err(|e| AppError::Database(e.to_string()))?;
                Some(PendingLogin {
                    member_id,
                    remember_me: remember_me != 0,
                    expires_at,
                })
            }
            None => None,
        })
    }

    /// Atomically validate and delete a pending-login. Returns the
    /// row's data on success, `None` if the token was missing /
    /// expired / already consumed. Use this from /login/totp after
    /// the code (or recovery code) has been verified.
    pub async fn consume(&self, token: &str) -> Result<Option<PendingLogin>> {
        let token_hash = hash_token(token);
        let now = Utc::now().naive_utc();

        let row: Option<(String, i32, DateTime<Utc>)> = sqlx::query_as(
            "DELETE FROM pending_logins \
             WHERE token_hash = ? AND expires_at > ? \
             RETURNING member_id, remember_me, expires_at",
        )
        .bind(&token_hash)
        .bind(now)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(match row {
            Some((member_id_str, remember_me, expires_at)) => {
                let member_id = Uuid::parse_str(&member_id_str)
                    .map_err(|e| AppError::Database(e.to_string()))?;
                Some(PendingLogin {
                    member_id,
                    remember_me: remember_me != 0,
                    expires_at,
                })
            }
            None => None,
        })
    }

    /// Wipe every pending-login for a member. Used during disable-2FA
    /// (so a stale cookie can't keep a half-login alive) and on
    /// successful login completion as a belt-and-suspenders cleanup.
    pub async fn delete_for_member(&self, member_id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM pending_logins WHERE member_id = ?")
            .bind(member_id.to_string())
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    /// Drop all expired pending-logins. Called by the periodic
    /// cleanup task that already prunes `sessions`.
    #[allow(dead_code)]
    pub async fn cleanup_expired(&self) -> Result<u64> {
        let result = sqlx::query("DELETE FROM pending_logins WHERE expires_at <= ?")
            .bind(Utc::now().naive_utc())
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(result.rows_affected())
    }
}

/// Build the cookie that carries the pending-login token. Same hardening
/// flags as the session cookie: HttpOnly, SameSite=Lax, Secure (when
/// HTTPS), 5-minute Max-Age. The narrow lifetime AND the separate cookie
/// name together mean a stolen `pending_login` cookie expires before
/// most attackers could automate use of it.
pub fn create_cookie(token: &str, secure: bool) -> Cookie<'static> {
    Cookie::build((COOKIE_NAME, token.to_string()))
        .path("/")
        .same_site(SameSite::Lax)
        .http_only(true)
        .secure(secure)
        .max_age(cookie::time::Duration::minutes(TTL.num_minutes()))
        .build()
}

pub fn create_clear_cookie() -> Cookie<'static> {
    Cookie::build((COOKIE_NAME, ""))
        .path("/")
        .same_site(SameSite::Lax)
        .http_only(true)
        .max_age(cookie::time::Duration::seconds(0))
        .build()
}

fn generate_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn hash_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = sha2::Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}
