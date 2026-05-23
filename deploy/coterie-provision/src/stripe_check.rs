use anyhow::{bail, Result};

/// Validate that a Stripe-style credential starts with the expected
/// prefix. Used to catch the common "pasted the wrong key" foot-gun
/// before we write it to .env and confuse the operator hours later.
pub fn validate_prefix(value: &str, expected: &str) -> Result<()> {
    if value.is_empty() {
        bail!("value is empty (expected prefix {expected}…)");
    }
    if !value.starts_with(expected) {
        bail!(
            "value does not start with {expected}… (got first 4 chars: {:?})",
            &value.chars().take(4).collect::<String>()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pk_test_ok() {
        validate_prefix("pk_test_abc123", "pk_test_").unwrap();
    }

    #[test]
    fn sk_test_ok() {
        validate_prefix("sk_test_abc123", "sk_test_").unwrap();
    }

    #[test]
    fn pk_live_ok() {
        validate_prefix("pk_live_abc123", "pk_live_").unwrap();
    }

    #[test]
    fn sk_live_ok() {
        validate_prefix("sk_live_abc123", "sk_live_").unwrap();
    }

    #[test]
    fn whsec_ok() {
        validate_prefix("whsec_abc123", "whsec_").unwrap();
    }

    #[test]
    fn wrong_prefix_errors() {
        assert!(validate_prefix("sk_test_abc", "pk_test_").is_err());
        assert!(validate_prefix("sk_live_abc", "sk_test_").is_err());
        assert!(validate_prefix("whsec_abc", "sk_live_").is_err());
    }

    #[test]
    fn empty_errors() {
        assert!(validate_prefix("", "sk_test_").is_err());
    }
}
