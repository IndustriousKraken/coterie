use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use argon2::password_hash::{SaltString, rand_core::OsRng};
use chrono::{Duration, Utc};
use cookie::{Cookie, SameSite};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    domain::Member,
    error::{AppError, Result},
};

pub mod csrf;
pub mod session;

use session::{Session, SessionStore};
pub use csrf::CsrfService;

pub struct AuthService {
    session_store: SessionStore,
}

impl AuthService {
    pub fn new(pool: SqlitePool, _secret: String) -> Self {
        // Note: secret parameter kept for API compatibility but not used.
        // Session security relies on cryptographically random tokens stored server-side,
        // not on signed tokens, so a signing secret isn't needed.
        Self {
            session_store: SessionStore::new(pool),
        }
    }

    pub async fn verify_password(password: &str, hash: &str) -> Result<bool> {
        let parsed_hash = PasswordHash::new(hash)
            .map_err(|e| AppError::Internal(format!("Invalid password hash: {}", e)))?;
        
        let argon2 = Argon2::default();
        
        Ok(argon2.verify_password(password.as_bytes(), &parsed_hash).is_ok())
    }

    /// Hash a password using Argon2. Used in tests and member creation.
    #[allow(dead_code)]
    pub async fn hash_password(password: &str) -> Result<String> {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        
        let password_hash = argon2
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| AppError::Internal(format!("Password hashing failed: {}", e)))?;
        
        Ok(password_hash.to_string())
    }

    pub async fn create_session(&self, member_id: Uuid, duration_hours: i64) -> Result<(Session, String)> {
        let token = generate_token();
        let expires_at = Utc::now() + Duration::hours(duration_hours);
        
        let session = self.session_store
            .create(member_id, &token, expires_at)
            .await?;
        
        Ok((session, token))
    }

    pub async fn validate_session(&self, token: &str) -> Result<Option<Session>> {
        self.session_store.find_by_token(token).await
    }

    pub async fn invalidate_session(&self, token: &str) -> Result<()> {
        self.session_store.delete_by_token(token).await
    }

    pub async fn cleanup_expired_sessions(&self) -> Result<u64> {
        self.session_store.cleanup_expired().await
    }

    pub fn create_session_cookie(&self, token: &str, secure: bool) -> Cookie<'static> {
        Cookie::build(("session", token.to_string()))
            .path("/")
            .same_site(SameSite::Lax)
            .http_only(true)
            .secure(secure)
            .max_age(cookie::time::Duration::hours(24))
            .build()
    }

    pub fn create_logout_cookie() -> Cookie<'static> {
        Cookie::build(("session", ""))
            .path("/")
            .same_site(SameSite::Lax)
            .http_only(true)
            .max_age(cookie::time::Duration::seconds(0))
            .build()
    }
}

fn generate_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub async fn get_password_hash(pool: &SqlitePool, email: &str) -> Result<Option<String>> {
    let result = sqlx::query_scalar::<_, String>(
        "SELECT password_hash FROM members WHERE email = ?"
    )
    .bind(email)
    .fetch_optional(pool)
    .await?;
    
    Ok(result)
}

pub async fn get_member_by_email(pool: &SqlitePool, email: &str) -> Result<Option<Member>> {
    use crate::repository::{MemberRepository, SqliteMemberRepository};
    
    let repo = SqliteMemberRepository::new(pool.clone());
    repo.find_by_email(email).await
}