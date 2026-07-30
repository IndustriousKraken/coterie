//! Structured logging for the authentication surface.
//!
//! Every auth outcome — login, second factor, logout, rate-limit trip,
//! password reset, password change, 2FA lifecycle — routes through the
//! three functions here so the emitted field set is identical at every
//! call site. An operator filters on `event` and `member_id`; nothing
//! here interpolates a value into the message string.
//!
//! Two rules this module exists to enforce:
//!
//! 1. **The log is more specific than the response.** `reason` names the
//!    real denial cause (`unknown_email` vs `bad_password` vs
//!    `inactive_status`); the HTTP response stays enumeration-safe.
//! 2. **Credentials never reach a log.** Passwords, reset tokens, session
//!    tokens, TOTP codes and recovery codes are never passed in. The
//!    submitted *identifier* is logged (investigating without it is not
//!    possible) but only after [`safe_identifier`] has confirmed it looks
//!    like an email — a password typed into the email field must not be
//!    persisted in a log that outlives the request.

use std::net::IpAddr;

use uuid::Uuid;

/// Stand-in for a submitted identifier that isn't a syntactically valid
/// email address. The most likely cause of one is a password typed into
/// the wrong field.
pub const REDACTED_IDENTIFIER: &str = "[redacted-not-an-email]";

/// Placeholder for a field with no value on this event. A literal keeps
/// the field set rectangular so a log query never has to handle both
/// "absent" and "empty".
const NONE: &str = "-";

/// The identifier as it is safe to log: the value itself when it parses
/// as an email, [`REDACTED_IDENTIFIER`] otherwise.
///
/// The web login form accepts a username *or* an email, so a legitimate
/// username gets redacted too. That is the intended trade: when the
/// member exists the event carries their `member_id` anyway, and when
/// they don't, an unrecognised non-email string is far more likely to be
/// a mistyped credential than a real username.
pub fn safe_identifier(identifier: &str) -> &str {
    let mut parts = identifier.split('@');
    let looks_like_email = matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some(local), Some(domain), None)
            if !local.is_empty()
                && domain.len() > 2
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
    ) && identifier.len() <= 254
        && !identifier.chars().any(char::is_whitespace);

    if looks_like_email {
        identifier
    } else {
        REDACTED_IDENTIFIER
    }
}

fn fmt_id(member_id: Option<Uuid>) -> String {
    member_id.map_or_else(|| NONE.to_string(), |id| id.to_string())
}

fn fmt_ip(ip: Option<IpAddr>) -> String {
    ip.map_or_else(|| NONE.to_string(), |ip| ip.to_string())
}

/// A successful (or otherwise non-denial) auth outcome, at `info`.
///
/// `detail` is a short machine-readable tag for the variant of the
/// outcome — `"recovery_code"`, `"2fa_required"` — never free prose and
/// never anything derived from a credential.
pub fn ok(
    event: &str,
    member_id: Option<Uuid>,
    ip: Option<IpAddr>,
    identifier: Option<&str>,
    detail: Option<&str>,
) {
    tracing::info!(
        event = %event,
        outcome = "ok",
        member_id = %fmt_id(member_id),
        ip = %fmt_ip(ip),
        reason = NONE,
        identifier = identifier.unwrap_or(NONE),
        detail = detail.unwrap_or(NONE),
        "{event}"
    );
}

/// A denied auth outcome, at `warn` — so an operator scanning warnings
/// sees the security-relevant subset without reading every sign-in.
///
/// `reason` is the specific cause the HTTP response deliberately hides.
pub fn denied(
    event: &str,
    reason: &str,
    member_id: Option<Uuid>,
    ip: Option<IpAddr>,
    identifier: Option<&str>,
    detail: Option<&str>,
) {
    tracing::warn!(
        event = %event,
        outcome = "denied",
        member_id = %fmt_id(member_id),
        ip = %fmt_ip(ip),
        reason = %reason,
        identifier = identifier.unwrap_or(NONE),
        detail = detail.unwrap_or(NONE),
        "{event}"
    );
}

/// `auth.password_rejected` — a submitted password failed the policy.
///
/// Carries the rule that failed and the submitted length, which the
/// length rules are otherwise undiagnosable without. A length is not a
/// credential; the password is, and it is not passed in here at all.
pub fn password_rejected(rule: &str, length: usize, member_id: Option<Uuid>, ip: Option<IpAddr>) {
    tracing::warn!(
        event = "auth.password_rejected",
        outcome = "denied",
        member_id = %fmt_id(member_id),
        ip = %fmt_ip(ip),
        reason = %rule,
        identifier = NONE,
        length,
        "auth.password_rejected"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_emails_are_logged_verbatim() {
        for ok in ["a@b.co", "member@example.com", "first.last+tag@sub.example"] {
            assert_eq!(safe_identifier(ok), ok);
        }
    }

    #[test]
    fn a_password_typed_into_the_email_field_is_redacted() {
        // The hazard this guards: no '@' at all, or an '@' inside what is
        // obviously a passphrase rather than an address.
        for bad in [
            "hunter2",
            "correct horse battery staple",
            "P@ssw0rd",
            "",
            "@example.com",
            "user@",
            "a@b@c.com",
        ] {
            assert_eq!(
                safe_identifier(bad),
                REDACTED_IDENTIFIER,
                "{bad:?} should have been redacted"
            );
        }
    }

    #[test]
    fn an_overlong_identifier_is_redacted_rather_than_stuffed_into_the_log() {
        let stuffed = format!("{}@example.com", "a".repeat(300));
        assert_eq!(safe_identifier(&stuffed), REDACTED_IDENTIFIER);
    }
}
