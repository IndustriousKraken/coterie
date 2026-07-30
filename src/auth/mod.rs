use argon2::password_hash::{rand_core::OsRng, SaltString};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use chrono::{Duration, Utc};
use cookie::{Cookie, SameSite};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    domain::Member,
    error::{AppError, Result},
};

pub mod csrf;
pub mod email_tokens;
pub mod pending_login;
pub mod recovery_codes;
pub mod secret_crypto;
pub mod session;
pub mod tokens;
pub mod totp;

pub use csrf::CsrfService;
pub use pending_login::PendingLoginService;
pub use secret_crypto::SecretCrypto;
use session::{Session, SessionStore};
pub use totp::TotpService;

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

        Ok(argon2
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok())
    }

    /// Run a full Argon2 hash so the caller's latency is
    /// indistinguishable from a real password check. Call this when
    /// the looked-up user does not exist to prevent timing-based
    /// username enumeration.
    pub async fn verify_dummy(password: &str) {
        // Hash (not verify) the password — this exercises the same Argon2
        // work factor as a real login attempt. We discard the result.
        let _ = Self::hash_password(password).await;
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

    pub async fn create_session(
        &self,
        member_id: Uuid,
        duration_hours: i64,
    ) -> Result<(Session, String)> {
        let token = tokens::generate_token();
        let expires_at = Utc::now() + Duration::hours(duration_hours);

        let session = self
            .session_store
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

    /// Invalidate ALL sessions for a given member. Call this on login (to
    /// prevent session-fixation), on privilege change (admin promotion /
    /// demotion), and on account status changes (suspend / expire) to
    /// force re-authentication with fresh permissions.
    ///
    /// The `auth.sessions_invalidated` event is emitted here rather than
    /// at the call sites: every sweep — login, password change, reset,
    /// admin status change — funnels through this method, so a caller
    /// cannot add a sweep that goes unrecorded. The caller's IP isn't
    /// known at this depth; the member id is what an operator needs to
    /// answer "why was I logged out".
    pub async fn invalidate_all_sessions(&self, member_id: Uuid) -> Result<()> {
        let result = self.session_store.delete_by_member(member_id).await;
        if result.is_ok() {
            crate::util::auth_log::ok(
                "auth.sessions_invalidated",
                Some(member_id),
                None,
                None,
                None,
            );
        }
        result
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

/// The password-policy rule a submitted password failed.
///
/// A named rule rather than a bare message so `auth.password_rejected`
/// can carry a stable machine-filterable `reason` — deriving one by
/// matching on the user-facing message would silently break the day
/// someone rewords it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordRule {
    TooShort,
    TooLong,
    NoUppercase,
    NoLowercase,
    NoDigit,
}

impl PasswordRule {
    /// The message shown to the person who typed the password.
    pub fn message(self) -> &'static str {
        match self {
            Self::TooShort => "Password must be at least 10 characters",
            Self::TooLong => "Password must be at most 128 characters",
            Self::NoUppercase => "Password must contain at least one uppercase letter",
            Self::NoLowercase => "Password must contain at least one lowercase letter",
            Self::NoDigit => "Password must contain at least one number",
        }
    }

    /// The `reason` field value in the log.
    pub fn slug(self) -> &'static str {
        match self {
            Self::TooShort => "too_short",
            Self::TooLong => "too_long",
            Self::NoUppercase => "no_uppercase",
            Self::NoLowercase => "no_lowercase",
            Self::NoDigit => "no_digit",
        }
    }
}

/// Validate password complexity. Returns `Ok(())` if the password meets
/// requirements, or the rule it failed.
pub fn validate_password(password: &str) -> std::result::Result<(), PasswordRule> {
    if password.len() < 10 {
        return Err(PasswordRule::TooShort);
    }
    // Upper bound guards against Argon2 CPU-amplification DoS: the Blake2b
    // pre-hash cost scales with input length, so an unauthenticated caller
    // must not be able to force hashing of an oversized password.
    if password.len() > 128 {
        return Err(PasswordRule::TooLong);
    }
    if !password.chars().any(|c| c.is_ascii_uppercase()) {
        return Err(PasswordRule::NoUppercase);
    }
    if !password.chars().any(|c| c.is_ascii_lowercase()) {
        return Err(PasswordRule::NoLowercase);
    }
    if !password.chars().any(|c| c.is_ascii_digit()) {
        return Err(PasswordRule::NoDigit);
    }
    Ok(())
}

/// Validate a submitted password and emit `auth.password_rejected` on
/// failure. Every `validate_password` call site on a request path goes
/// through this so the rejection can't be silently dropped; the event is
/// log-only (its volume is attacker-controlled, so no `audit_logs` row).
pub fn validate_password_logged(
    password: &str,
    member_id: Option<Uuid>,
    ip: Option<std::net::IpAddr>,
) -> std::result::Result<(), PasswordRule> {
    if let Err(rule) = validate_password(password) {
        crate::util::auth_log::password_rejected(rule.slug(), password.len(), member_id, ip);
        return Err(rule);
    }
    Ok(())
}

pub async fn get_password_hash(pool: &SqlitePool, email: &str) -> Result<Option<String>> {
    let result =
        sqlx::query_scalar::<_, String>("SELECT password_hash FROM members WHERE email = ?")
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

#[cfg(test)]
mod tests {
    use super::{validate_password, PasswordRule};

    // A valid complexity prefix ("Aa1") padded with lowercase 'a' to `len`.
    fn valid_of_len(len: usize) -> String {
        let mut s = String::from("Aa1");
        s.push_str(&"a".repeat(len - s.len()));
        s
    }

    #[test]
    fn accepts_128_rejects_129() {
        assert!(validate_password(&valid_of_len(128)).is_ok());
        assert_eq!(
            validate_password(&valid_of_len(129)),
            Err(PasswordRule::TooLong)
        );
    }

    #[test]
    fn rejects_multi_kilobyte_password() {
        // Argon2 DoS guard: an oversized input is rejected before hashing.
        assert_eq!(
            validate_password(&valid_of_len(10_000)),
            Err(PasswordRule::TooLong)
        );
    }

    #[test]
    fn each_rule_has_a_distinct_log_slug_and_message() {
        // The whole point of the enum: an operator filtering on `reason`
        // can tell the five rejections apart.
        let all = [
            PasswordRule::TooShort,
            PasswordRule::TooLong,
            PasswordRule::NoUppercase,
            PasswordRule::NoLowercase,
            PasswordRule::NoDigit,
        ];
        let slugs: std::collections::HashSet<_> = all.iter().map(|r| r.slug()).collect();
        assert_eq!(slugs.len(), all.len());

        assert_eq!(
            validate_password("Aa1").unwrap_err(),
            PasswordRule::TooShort
        );
        assert_eq!(
            validate_password("aa1aa1aa1aa1").unwrap_err(),
            PasswordRule::NoUppercase
        );
        assert_eq!(
            validate_password("AA1AA1AA1AA1").unwrap_err(),
            PasswordRule::NoLowercase
        );
        assert_eq!(
            validate_password("AaAaAaAaAaAa").unwrap_err(),
            PasswordRule::NoDigit
        );
    }
}
