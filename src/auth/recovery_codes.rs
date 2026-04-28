//! TOTP recovery codes — one-time use passwords for the case where a
//! member loses their authenticator app. 10 codes per enrollment,
//! generated server-side, displayed exactly once at the end of
//! enrollment (and on regenerate).
//!
//! Codes are stored argon2-hashed in `members.totp_recovery_codes` as
//! a JSON array. Verification iterates the array and consumes the
//! matching entry by rewriting the JSON without it. Constant-time
//! compare isn't strictly needed (argon2 is the heavy operation) but
//! we still avoid early-exits that could leak whether SOME code
//! matched vs WHICH one — by walking every entry.

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use argon2::password_hash::{SaltString, rand_core::OsRng};
use rand::seq::SliceRandom;
use rand::RngCore;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::error::{AppError, Result};

/// 10 codes per enrollment. Standard issue across most 2FA-enabled
/// services (Stripe, GitHub, Google, etc.) — striking a balance
/// between "enough to survive losing a phone" and "not so many that
/// users leave them lying around in unprotected places."
pub const RECOVERY_CODE_COUNT: usize = 10;

/// 4-4-4 hyphen-separated, drawn from a base32-ish alphabet that omits
/// look-alikes (no 0/O, no 1/I/L). Long enough to brute-force-resist
/// (32^12 ≈ 1.15e18) without being unreadable.
const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTVWXYZ23456789";
const GROUP_LEN: usize = 4;
const GROUPS: usize = 3;

pub struct GeneratedCodes {
    /// Plaintext codes. Caller must show these to the user EXACTLY ONCE
    /// (they can't be recovered after this — the DB has only hashes).
    pub plaintext: Vec<String>,
    /// What gets persisted in `members.totp_recovery_codes` (JSON
    /// array of argon2 hashes). Caller writes it.
    pub stored_json: String,
}

/// Generate a fresh batch of codes, hash them, and return both the
/// plaintext set (to display once) and the JSON blob to persist.
pub fn generate() -> Result<GeneratedCodes> {
    let plaintext: Vec<String> = (0..RECOVERY_CODE_COUNT)
        .map(|_| random_code())
        .collect();
    let hashes: Vec<String> = plaintext
        .iter()
        .map(|c| argon2_hash(c))
        .collect::<Result<Vec<_>>>()?;
    let stored_json = serde_json::to_string(&hashes)
        .map_err(|e| AppError::Internal(format!("Recovery codes serialize failed: {}", e)))?;
    Ok(GeneratedCodes { plaintext, stored_json })
}

/// Generate codes AND persist them to the member's row. Used by
/// enrollment confirmation and the regenerate path. Returns the
/// plaintext set for one-time display.
pub async fn issue_for_member(
    pool: &SqlitePool,
    member_id: Uuid,
) -> Result<Vec<String>> {
    let codes = generate()?;
    sqlx::query(
        "UPDATE members SET totp_recovery_codes = ?, \
                            updated_at = CURRENT_TIMESTAMP \
         WHERE id = ?",
    )
    .bind(&codes.stored_json)
    .bind(member_id.to_string())
    .execute(pool)
    .await
    .map_err(|e| AppError::Database(e.to_string()))?;
    Ok(codes.plaintext)
}

/// Try to consume a recovery code submitted by the user during the
/// TOTP step of login. Walks every stored hash (no early exit on
/// match — see module doc) and on a successful match rewrites the
/// JSON without the consumed hash. Returns whether a code was
/// consumed.
///
/// Race-free: the SELECT + UPDATE happen inside a transaction with
/// SQLite's per-row write lock, so two concurrent calls with the
/// same code can't both succeed — the second sees the rewritten JSON.
pub async fn try_consume(
    pool: &SqlitePool,
    member_id: Uuid,
    submitted: &str,
) -> Result<bool> {
    let normalized = normalize(submitted);
    if normalized.is_empty() {
        return Ok(false);
    }

    let mut tx = pool.begin().await
        .map_err(|e| AppError::Database(e.to_string()))?;

    let json_opt: Option<String> = sqlx::query_scalar(
        "SELECT totp_recovery_codes FROM members WHERE id = ?",
    )
    .bind(member_id.to_string())
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| AppError::Database(e.to_string()))?
    .flatten();

    let json = match json_opt {
        Some(s) if !s.is_empty() => s,
        _ => {
            tx.commit().await.map_err(|e| AppError::Database(e.to_string()))?;
            return Ok(false);
        }
    };

    let mut hashes: Vec<String> = serde_json::from_str(&json)
        .map_err(|e| AppError::Internal(format!("Recovery codes parse failed: {}", e)))?;

    // Walk every entry; record the index of the match. Iterate fully
    // so the time-cost is constant per call, regardless of which
    // code was submitted.
    let mut matched_idx: Option<usize> = None;
    for (i, hash) in hashes.iter().enumerate() {
        if argon2_verify(&normalized, hash) && matched_idx.is_none() {
            matched_idx = Some(i);
        }
    }

    let consumed = match matched_idx {
        Some(i) => {
            hashes.remove(i);
            let new_json = serde_json::to_string(&hashes)
                .map_err(|e| AppError::Internal(format!("Recovery codes reserialize failed: {}", e)))?;
            sqlx::query(
                "UPDATE members SET totp_recovery_codes = ?, \
                                    updated_at = CURRENT_TIMESTAMP \
                 WHERE id = ?",
            )
            .bind(&new_json)
            .bind(member_id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
            true
        }
        None => false,
    };

    tx.commit().await.map_err(|e| AppError::Database(e.to_string()))?;
    Ok(consumed)
}

/// How many recovery codes the member has left. Shown on the security
/// page so they know when to regenerate.
pub async fn remaining_count(pool: &SqlitePool, member_id: Uuid) -> Result<usize> {
    let json: Option<String> = sqlx::query_scalar(
        "SELECT totp_recovery_codes FROM members WHERE id = ?",
    )
    .bind(member_id.to_string())
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::Database(e.to_string()))?
    .flatten();
    let json = match json {
        Some(s) if !s.is_empty() => s,
        _ => return Ok(0),
    };
    let hashes: Vec<String> = serde_json::from_str(&json)
        .map_err(|e| AppError::Internal(format!("Recovery codes parse failed: {}", e)))?;
    Ok(hashes.len())
}

/// Render a code group-separated for display: "ABCD-EFGH-JKMN".
pub fn pretty(code: &str) -> String {
    let normalized = normalize(code);
    normalized
        .as_bytes()
        .chunks(GROUP_LEN)
        .map(|c| std::str::from_utf8(c).unwrap_or(""))
        .collect::<Vec<_>>()
        .join("-")
}

// --------------------------------------------------------------------
// Internals
// --------------------------------------------------------------------

fn random_code() -> String {
    let mut rng = rand::thread_rng();
    let mut buf = [0u8; GROUPS * GROUP_LEN];
    rng.fill_bytes(&mut buf);
    let chars: String = buf
        .iter()
        .map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char)
        .collect();
    // Hyphenate for display. We strip hyphens and uppercase before
    // hashing/verifying, so the stored form is irrelevant to the
    // user-facing format.
    pretty(&chars)
}

/// Strip whitespace + hyphens, uppercase. The user might paste with
/// spaces, lowercase, etc. — we accept all those.
fn normalize(input: &str) -> String {
    input
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_uppercase()
}

/// We hash and verify the *normalized* form so display formatting
/// (hyphens, case) doesn't affect equality. Random codes already use
/// only the ALPHABET set.
fn argon2_hash(code: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let normalized = normalize(code);
    let h = argon2
        .hash_password(normalized.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(format!("Recovery code hash failed: {}", e)))?;
    Ok(h.to_string())
}

fn argon2_verify(normalized_code: &str, hash: &str) -> bool {
    let parsed = match PasswordHash::new(hash) {
        Ok(p) => p,
        Err(_) => return false,
    };
    Argon2::default()
        .verify_password(normalized_code.as_bytes(), &parsed)
        .is_ok()
}

/// Used only by tests; production is not expected to need a no-arg
/// shuffle. Kept here so the test-only helper doesn't pollute the
/// main API.
#[cfg(test)]
fn shuffled<T>(mut v: Vec<T>) -> Vec<T> {
    v.shuffle(&mut rand::thread_rng());
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_yields_ten_unique_codes() {
        let codes = generate().expect("generate");
        assert_eq!(codes.plaintext.len(), RECOVERY_CODE_COUNT);
        let unique: std::collections::HashSet<_> = codes.plaintext.iter().collect();
        assert_eq!(unique.len(), RECOVERY_CODE_COUNT, "codes must not collide");
    }

    #[test]
    fn codes_are_pretty_format() {
        let codes = generate().expect("generate");
        for code in &codes.plaintext {
            assert_eq!(code.len(), GROUPS * GROUP_LEN + (GROUPS - 1)); // "XXXX-XXXX-XXXX"
            assert_eq!(code.chars().filter(|c| *c == '-').count(), GROUPS - 1);
        }
    }

    #[test]
    fn normalize_strips_format() {
        assert_eq!(normalize(" abcd-EFGH-jkmn "), "ABCDEFGHJKMN");
        assert_eq!(normalize("ABCDEFGHJKMN"), "ABCDEFGHJKMN");
    }

    #[test]
    fn hash_and_verify_round_trip() {
        let code = "ABCD-EFGH-JKMN";
        let hash = argon2_hash(code).expect("hash");
        assert!(argon2_verify(&normalize(code), &hash));
        // Whitespace / case variants normalize to the same string.
        assert!(argon2_verify(&normalize("abcd efgh jkmn"), &hash));
        assert!(!argon2_verify("ZZZZZZZZZZZZ", &hash));
    }

    #[test]
    fn shuffle_keeps_set_intact() {
        let codes = generate().expect("generate");
        let shuffled = shuffled(codes.plaintext.clone());
        let a: std::collections::HashSet<_> = codes.plaintext.iter().collect();
        let b: std::collections::HashSet<_> = shuffled.iter().collect();
        assert_eq!(a, b);
    }
}
