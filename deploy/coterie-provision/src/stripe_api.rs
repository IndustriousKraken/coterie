use anyhow::{anyhow, Context, Result};
use secrecy::{ExposeSecret, Secret};
use std::time::Duration;

/// Abstraction over the Stripe API smoke test. Production calls
/// `https://api.stripe.com/v1/balance`; tests use `FakeStripeApi` to
/// script success/failure responses.
pub trait StripeApi {
    fn check_balance(&self, secret_key: &Secret<String>) -> Result<()>;
}

pub struct RealStripeApi {
    client: reqwest::blocking::Client,
}

impl RealStripeApi {
    pub fn new() -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .context("building reqwest client for Stripe smoke test")?;
        Ok(Self { client })
    }
}

impl StripeApi for RealStripeApi {
    fn check_balance(&self, secret_key: &Secret<String>) -> Result<()> {
        let response = self
            .client
            .get("https://api.stripe.com/v1/balance")
            .basic_auth(secret_key.expose_secret(), Some(""))
            .send()
            .context("HTTP request to Stripe /v1/balance failed")?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().unwrap_or_default();
            return Err(anyhow!(
                "Stripe /v1/balance returned HTTP {} — key rejected.\nResponse: {}",
                status.as_u16(),
                body.chars().take(500).collect::<String>()
            ));
        }
        Ok(())
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod fake {
    use super::*;
    use std::cell::RefCell;

    /// Configurable response policy for `FakeStripeApi`.
    #[derive(Debug, Clone, Copy)]
    pub enum FakeStripePolicy {
        /// Every call returns Ok(()).
        AcceptAll,
        /// Every call returns Err(...) — for the abort-before-mutation path.
        RejectAll,
    }

    pub struct FakeStripeApi {
        pub attempted: RefCell<Vec<String>>,
        pub policy: FakeStripePolicy,
    }

    impl FakeStripeApi {
        pub fn accept_all() -> Self {
            Self {
                attempted: RefCell::new(Vec::new()),
                policy: FakeStripePolicy::AcceptAll,
            }
        }

        pub fn reject_all() -> Self {
            Self {
                attempted: RefCell::new(Vec::new()),
                policy: FakeStripePolicy::RejectAll,
            }
        }
    }

    impl StripeApi for FakeStripeApi {
        fn check_balance(&self, secret_key: &Secret<String>) -> Result<()> {
            self.attempted
                .borrow_mut()
                .push(secret_key.expose_secret().clone());
            match self.policy {
                FakeStripePolicy::AcceptAll => Ok(()),
                FakeStripePolicy::RejectAll => {
                    Err(anyhow!("FakeStripeApi: configured to reject (HTTP 401)"))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeStripeApi;
    use super::*;

    #[test]
    fn accept_all_returns_ok() {
        let api = FakeStripeApi::accept_all();
        let key = Secret::new("sk_live_test".to_string());
        api.check_balance(&key).unwrap();
        assert_eq!(api.attempted.borrow().len(), 1);
    }

    #[test]
    fn reject_all_returns_err() {
        let api = FakeStripeApi::reject_all();
        let key = Secret::new("sk_live_bad".to_string());
        let err = api.check_balance(&key).unwrap_err();
        assert!(err.to_string().contains("reject"));
    }
}
