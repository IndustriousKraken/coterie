//! TOTP (RFC 6238) for two-factor authentication.
//!
//! Persistence lives on the `members` table:
//!   - `totp_secret_encrypted` — the shared secret, ChaCha20-Poly1305-
//!     encrypted with `SecretCrypto`. Stealing the DB on its own does
//!     not yield workable secrets. Same encryption key as SMTP /
//!     Discord secrets.
//!   - `totp_enabled_at` — null when 2FA is off; otherwise the
//!     enrollment timestamp. The login flow reads only this column to
//!     decide whether to require the second step.
//!   - `totp_recovery_codes` — JSON array of argon2-hashed one-time
//!     codes. Owned by `recovery_codes.rs`.
//!
//! Convention: callers hold the `TotpService` (constructed once at
//! startup), pass the `Pool` and per-request data in. There's no
//! global state.

use std::sync::Arc;

use rand::RngCore;
use sqlx::SqlitePool;
use totp_rs::{Algorithm, TOTP};
use uuid::Uuid;

use crate::{
    auth::SecretCrypto,
    error::{AppError, Result},
};

/// 20 bytes (160 bits) — the RFC 6238 minimum for SHA-1 and what
/// every popular authenticator app expects. Bumping past 32 bytes can
/// confuse some apps, so we deliberately stay at the standard.
const SECRET_LEN: usize = 20;

/// SHA-1 / 6 digits / 30s. Anything else and Google Authenticator,
/// Authy, 1Password, etc. will silently produce the wrong code.
const ALGO: Algorithm = Algorithm::SHA1;
const DIGITS: usize = 6;
const STEP: u64 = 30;

/// One step on each side of "now" — accommodates ~30s of clock skew.
/// Bigger skew means less defense against replay; smaller means real
/// users with slightly-wrong phone clocks get rejected.
const SKEW: u8 = 1;

pub struct TotpService {
    crypto: Arc<SecretCrypto>,
    pool: SqlitePool,
    /// Shown in the authenticator app's account list. Pulled from
    /// `org.name` setting at construction time so it tracks the
    /// configured org name without a DB lookup per enrollment.
    issuer: String,
}

/// What the enrollment-start handler returns to the page so it can
/// render the QR + manual key. The secret is plaintext (base32) here
/// because it has to round-trip back from the user's confirmation
/// POST so we can save it once a TOTP code verifies — the page hides
/// it inside a hidden field.
pub struct EnrollmentInit {
    /// `otpauth://totp/...?secret=...&issuer=...` for the QR code.
    pub otpauth_url: String,
    /// Pretty base32 secret for users who can't scan a QR (manual entry).
    pub secret_base32: String,
    /// SVG markup for an inline `<img src="data:..."/>`-free QR. The
    /// page can drop it directly into the DOM.
    pub qr_svg: String,
}

impl TotpService {
    pub fn new(pool: SqlitePool, crypto: Arc<SecretCrypto>, issuer: String) -> Self {
        Self { pool, crypto, issuer }
    }

    /// Generate a fresh TOTP secret and the artifacts the enrollment
    /// page needs to display. The secret is NOT persisted yet —
    /// persistence happens only after `confirm_enrollment` verifies
    /// the user can produce a valid code from it. This way an
    /// abandoned enrollment leaves no DB trace.
    pub fn begin_enrollment(&self, account_name: &str) -> Result<EnrollmentInit> {
        let secret = random_secret();
        let totp = build_totp(secret.clone(), &self.issuer, account_name)?;

        let otpauth_url = totp.get_url();
        let secret_base32 = totp.get_secret_base32();
        let qr_svg = render_qr_svg(&otpauth_url)?;

        Ok(EnrollmentInit { otpauth_url, secret_base32, qr_svg })
    }

    /// Verify the supplied TOTP code against `secret_base32` and, on
    /// success, persist the encrypted secret + stamp `totp_enabled_at`.
    /// Returns whether enrollment succeeded.
    ///
    /// Recovery-code generation is the caller's responsibility (see
    /// `recovery_codes::issue_for_member`) — split that way so the
    /// security-page handler can return the freshly-generated codes
    /// to the user atomically with enrollment confirmation.
    pub async fn confirm_enrollment(
        &self,
        member_id: Uuid,
        secret_base32: &str,
        code: &str,
        account_name: &str,
    ) -> Result<bool> {
        let secret = decode_base32(secret_base32)?;
        let totp = build_totp(secret.clone(), &self.issuer, account_name)?;
        if !check_code(&totp, code) {
            return Ok(false);
        }

        // Encrypt the plaintext secret (base32 form) so we can
        // round-trip it later via the same path as decode in
        // `verify_for_member`.
        let encrypted = self.crypto.encrypt(secret_base32)?;

        sqlx::query(
            "UPDATE members \
             SET totp_secret_encrypted = ?, \
                 totp_enabled_at = CURRENT_TIMESTAMP, \
                 updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?",
        )
        .bind(&encrypted)
        .bind(member_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(true)
    }

    /// Verify a TOTP code against this member's stored secret. Returns
    /// `Ok(false)` if 2FA isn't enabled, the secret is missing /
    /// undecryptable, or the code doesn't match. Distinct error vs
    /// false-result: only Database / Internal errors bubble up; bad
    /// codes are not errors.
    pub async fn verify_for_member(
        &self,
        member_id: Uuid,
        code: &str,
        account_name: &str,
    ) -> Result<bool> {
        let row: Option<(Option<String>, Option<chrono::NaiveDateTime>)> = sqlx::query_as(
            "SELECT totp_secret_encrypted, totp_enabled_at FROM members WHERE id = ?",
        )
        .bind(member_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        let (encrypted_opt, enabled_at) = match row {
            Some(t) => t,
            None => return Ok(false),
        };
        if enabled_at.is_none() {
            return Ok(false);
        }
        let encrypted = match encrypted_opt {
            Some(s) if !s.is_empty() => s,
            _ => return Ok(false),
        };

        // Don't propagate decrypt errors — they happen when the
        // session_secret rotates. Treat as "no valid secret on file".
        let secret_base32 = match self.crypto.decrypt(&encrypted) {
            Ok(s) => s,
            Err(_) => return Ok(false),
        };
        let secret = decode_base32(&secret_base32)?;
        let totp = build_totp(secret, &self.issuer, account_name)?;
        Ok(check_code(&totp, code))
    }

    /// Wipe TOTP state for a member: clears the secret, `totp_enabled_at`,
    /// and recovery codes. Caller must have already verified a current
    /// TOTP code (or recovery code) before invoking this — disable is
    /// an authenticated action. Bulk-clears `pending_logins` too in
    /// case any half-finished login was hanging around.
    pub async fn disable(&self, member_id: Uuid) -> Result<()> {
        let mut tx = self.pool.begin().await
            .map_err(|e| AppError::Database(e.to_string()))?;

        sqlx::query(
            "UPDATE members \
             SET totp_secret_encrypted = NULL, \
                 totp_enabled_at = NULL, \
                 totp_recovery_codes = NULL, \
                 updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?",
        )
        .bind(member_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        sqlx::query("DELETE FROM pending_logins WHERE member_id = ?")
            .bind(member_id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        tx.commit().await.map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    /// Whether 2FA is enabled for this member.
    pub async fn is_enabled(&self, member_id: Uuid) -> Result<bool> {
        let enabled: Option<chrono::NaiveDateTime> = sqlx::query_scalar(
            "SELECT totp_enabled_at FROM members WHERE id = ?",
        )
        .bind(member_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?
        .flatten();
        Ok(enabled.is_some())
    }
}

// --------------------------------------------------------------------
// Helpers
// --------------------------------------------------------------------

fn random_secret() -> Vec<u8> {
    let mut bytes = vec![0u8; SECRET_LEN];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes
}

fn build_totp(secret: Vec<u8>, issuer: &str, account_name: &str) -> Result<TOTP> {
    TOTP::new(
        ALGO,
        DIGITS,
        SKEW,
        STEP,
        secret,
        Some(issuer.to_string()),
        account_name.to_string(),
    )
    .map_err(|e| AppError::Internal(format!("TOTP construction failed: {}", e)))
}

fn check_code(totp: &TOTP, code: &str) -> bool {
    let trimmed: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    if trimmed.len() != DIGITS || !trimmed.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    totp.check_current(&trimmed).unwrap_or(false)
}

fn decode_base32(s: &str) -> Result<Vec<u8>> {
    use totp_rs::Secret;
    Secret::Encoded(s.to_string())
        .to_bytes()
        .map_err(|e| AppError::Internal(format!("Invalid TOTP secret encoding: {:?}", e)))
}

fn render_qr_svg(otpauth_url: &str) -> Result<String> {
    use qrcode::{render::svg, QrCode};
    let code = QrCode::new(otpauth_url.as_bytes())
        .map_err(|e| AppError::Internal(format!("QR encode failed: {}", e)))?;
    let svg = code
        .render::<svg::Color>()
        .min_dimensions(220, 220)
        // Inherit foreground/background from CSS so dark mode works
        // without a second render — the page can override via
        // `svg path { fill: ... }`.
        .dark_color(svg::Color("#111111"))
        .light_color(svg::Color("#ffffff"))
        .build();
    Ok(svg)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a TOTP from a fresh random secret without going through
    /// `TotpService` — the service tests are integration-tested in
    /// `tests/totp_test.rs` against a real DB. Here we only exercise
    /// the pure helpers (random_secret, build_totp, check_code,
    /// decode_base32, render_qr_svg) which don't need a pool.
    fn fresh_totp_with_secret() -> (TOTP, Vec<u8>) {
        let secret = random_secret();
        let totp = build_totp(secret.clone(), "Coterie Test", "alice@example.com").unwrap();
        (totp, secret)
    }

    #[test]
    fn random_secret_is_correct_length() {
        assert_eq!(random_secret().len(), SECRET_LEN);
    }

    #[test]
    fn url_and_qr_render() {
        let (totp, _) = fresh_totp_with_secret();
        let url = totp.get_url();
        assert!(url.starts_with("otpauth://totp/"));
        let svg = render_qr_svg(&url).unwrap();
        assert!(svg.contains("<svg"));
    }

    #[test]
    fn base32_round_trip() {
        let (totp, _) = fresh_totp_with_secret();
        let b32 = totp.get_secret_base32();
        let bytes = decode_base32(&b32).unwrap();
        assert_eq!(bytes.len(), SECRET_LEN);
    }

    #[test]
    fn check_code_rejects_garbage() {
        let (totp, _) = fresh_totp_with_secret();
        assert!(!check_code(&totp, "abcdef"));
        assert!(!check_code(&totp, "12345")); // wrong length
        assert!(!check_code(&totp, "        ")); // empty after trim
    }

    #[test]
    fn check_code_accepts_current_token() {
        let (totp, _) = fresh_totp_with_secret();
        let code = totp.generate_current().unwrap();
        assert!(check_code(&totp, &code));
        // Whitespace tolerance — users sometimes paste with spaces.
        let with_spaces = format!("{} {}", &code[..3], &code[3..]);
        assert!(check_code(&totp, &with_spaces));
    }
}
