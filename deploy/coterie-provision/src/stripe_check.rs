use anyhow::{bail, Result};

/// Validate that `value` begins with the expected Stripe-style prefix.
/// `expected` is one of `pk_test_`, `sk_test_`, `pk_live_`, `sk_live_`,
/// `whsec_`. The check is a strict prefix match — we do not validate
/// the body of the key, only that the operator pasted the right kind.
pub fn validate_prefix(value: &str, expected: &str) -> Result<()> {
    if !is_known_prefix(expected) {
        bail!("internal: unknown Stripe prefix `{expected}`");
    }
    if value.starts_with(expected) {
        Ok(())
    } else {
        bail!(
            "expected value to start with `{expected}` — got: `{}…`",
            value.chars().take(12).collect::<String>()
        )
    }
}

fn is_known_prefix(s: &str) -> bool {
    matches!(
        s,
        "pk_test_" | "sk_test_" | "pk_live_" | "sk_live_" | "whsec_"
    )
}

/// Convenience wrapper: returns true if a value looks like a webhook
/// signing secret (`whsec_`).
pub fn looks_like_webhook_secret(value: &str) -> bool {
    value.starts_with("whsec_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_pk_test() {
        validate_prefix("pk_test_abc123", "pk_test_").unwrap();
    }

    #[test]
    fn accepts_sk_test() {
        validate_prefix("sk_test_abc", "sk_test_").unwrap();
    }

    #[test]
    fn accepts_pk_live() {
        validate_prefix("pk_live_abc", "pk_live_").unwrap();
    }

    #[test]
    fn accepts_sk_live() {
        validate_prefix("sk_live_abc", "sk_live_").unwrap();
    }

    #[test]
    fn accepts_whsec() {
        validate_prefix("whsec_abc", "whsec_").unwrap();
    }

    #[test]
    fn rejects_wrong_prefix() {
        let err = validate_prefix("sk_live_abc", "sk_test_").unwrap_err();
        assert!(err.to_string().contains("sk_test_"));
    }

    #[test]
    fn rejects_empty() {
        let err = validate_prefix("", "sk_test_").unwrap_err();
        assert!(err.to_string().contains("sk_test_"));
    }

    #[test]
    fn rejects_unknown_expected() {
        let err = validate_prefix("anything", "xyz_").unwrap_err();
        assert!(err.to_string().contains("unknown Stripe prefix"));
    }
}
