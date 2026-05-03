//! Cloudflare Turnstile (or compatible) bot-challenge verification.
//!
//! Sits in front of `POST /public/signup` and `POST /public/donate` —
//! the two CSRF-exempt public endpoints that have side effects an
//! attacker would care about (carding via Stripe Checkout, fake-account
//! mass signup). Per-IP rate limiting catches single-source bursts;
//! this catches distributed bots.
//!
//! Failure mode is **fail closed**: when the org has configured a
//! provider, every request must carry a token that the provider says
//! is valid, or it gets a 403. The `Disabled` variant is the explicit
//! opt-out for local dev / orgs that haven't configured a provider yet.
//!
//! The verifier is a trait so tests can substitute a fake without
//! standing up an HTTP mock.
//!
//! See `src/config/mod.rs` `BotChallengeConfig` for the config shape
//! and the design notes in
//! `openspec/changes/bot-protection-public-apis/` for the full
//! rationale.
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::Deserialize;

use crate::config::BotChallengeConfig;

/// Why a verification call didn't pass. Each variant maps to a
/// distinct `outcome` in the structured log so an admin watching
/// tracing can tell "no token" apart from "provider down."
#[derive(Debug)]
pub enum VerifyError {
    /// Caller supplied no token but the provider is active. Token
    /// was either omitted from the JSON body or empty.
    Missing,
    /// Provider returned `success: false`. Carries the provider's
    /// own error code(s) for observability.
    Invalid { provider_codes: Vec<String> },
    /// Couldn't reach the provider within the configured timeout, or
    /// the provider returned a non-2xx HTTP response.
    ProviderUnreachable,
}

#[async_trait]
pub trait BotChallengeVerifier: Send + Sync {
    /// Verify `token` for a request bound for `route`. `client_ip` is
    /// passed through to the provider when present (some providers use
    /// it for rate-limit bookkeeping on their side).
    async fn verify(
        &self,
        route: &'static str,
        token: Option<&str>,
        client_ip: Option<IpAddr>,
    ) -> Result<(), VerifyError>;
}

/// No-op verifier. Used when `bot_challenge.provider = "disabled"`.
/// Ignores the token (whether present or not) and returns `Ok`.
/// Emits a debug-level trace so the bypass is visible if you crank
/// log levels.
pub struct DisabledVerifier;

#[async_trait]
impl BotChallengeVerifier for DisabledVerifier {
    async fn verify(
        &self,
        route: &'static str,
        _token: Option<&str>,
        _client_ip: Option<IpAddr>,
    ) -> Result<(), VerifyError> {
        tracing::debug!(route = route, outcome = "skipped", "bot_challenge");
        Ok(())
    }
}

/// Verifier that POSTs to a Turnstile-style `siteverify` endpoint.
/// Cloudflare and hCaptcha both speak this shape; the URL is
/// configurable so swapping providers is a settings change.
pub struct TurnstileVerifier {
    client: reqwest::Client,
    secret_key: String,
    verification_url: String,
    timeout: Duration,
}

impl TurnstileVerifier {
    pub fn new(client: reqwest::Client, cfg: &BotChallengeConfig) -> Self {
        Self {
            client,
            secret_key: cfg.secret_key.clone(),
            verification_url: cfg.verification_url.clone(),
            timeout: Duration::from_millis(cfg.timeout_ms),
        }
    }
}

#[derive(Debug, Deserialize)]
struct SiteverifyResponse {
    success: bool,
    #[serde(rename = "error-codes", default)]
    error_codes: Vec<String>,
}

#[async_trait]
impl BotChallengeVerifier for TurnstileVerifier {
    async fn verify(
        &self,
        route: &'static str,
        token: Option<&str>,
        client_ip: Option<IpAddr>,
    ) -> Result<(), VerifyError> {
        let started = Instant::now();
        let token = match token.filter(|s| !s.is_empty()) {
            Some(t) => t,
            None => {
                tracing::info!(
                    route = route,
                    outcome = "missing",
                    latency_ms = 0u128,
                    "bot_challenge",
                );
                return Err(VerifyError::Missing);
            }
        };

        let mut form: Vec<(&str, String)> = vec![
            ("secret", self.secret_key.clone()),
            ("response", token.to_string()),
        ];
        if let Some(ip) = client_ip {
            form.push(("remoteip", ip.to_string()));
        }

        let resp = match tokio::time::timeout(
            self.timeout,
            self.client.post(&self.verification_url).form(&form).send(),
        )
        .await
        {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                tracing::warn!(
                    route = route,
                    outcome = "provider_unreachable",
                    latency_ms = started.elapsed().as_millis() as u64,
                    error = %e,
                    "bot_challenge",
                );
                return Err(VerifyError::ProviderUnreachable);
            }
            Err(_timeout) => {
                tracing::warn!(
                    route = route,
                    outcome = "provider_unreachable",
                    latency_ms = started.elapsed().as_millis() as u64,
                    error = "timeout",
                    "bot_challenge",
                );
                return Err(VerifyError::ProviderUnreachable);
            }
        };

        if !resp.status().is_success() {
            tracing::warn!(
                route = route,
                outcome = "provider_unreachable",
                latency_ms = started.elapsed().as_millis() as u64,
                status = resp.status().as_u16(),
                "bot_challenge",
            );
            return Err(VerifyError::ProviderUnreachable);
        }

        let body: SiteverifyResponse = match resp.json().await {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(
                    route = route,
                    outcome = "provider_unreachable",
                    latency_ms = started.elapsed().as_millis() as u64,
                    error = %e,
                    "bot_challenge",
                );
                return Err(VerifyError::ProviderUnreachable);
            }
        };

        if body.success {
            tracing::info!(
                route = route,
                outcome = "pass",
                latency_ms = started.elapsed().as_millis() as u64,
                "bot_challenge",
            );
            Ok(())
        } else {
            tracing::info!(
                route = route,
                outcome = "invalid",
                latency_ms = started.elapsed().as_millis() as u64,
                provider_codes = ?body.error_codes,
                "bot_challenge",
            );
            Err(VerifyError::Invalid { provider_codes: body.error_codes })
        }
    }
}

/// Pick the verifier implementation that matches the loaded config.
/// Unknown provider strings fall back to `Disabled` with a startup
/// warning rather than refusing to boot — wrong-provider-name on a
/// production deployment is an operator error worth surfacing, but
/// the verifier itself isn't the right place to refuse to start the
/// whole app.
pub fn from_config(
    cfg: &BotChallengeConfig,
    client: reqwest::Client,
) -> Arc<dyn BotChallengeVerifier> {
    match cfg.provider.as_str() {
        "turnstile" | "hcaptcha" => {
            if cfg.secret_key.is_empty() {
                tracing::warn!(
                    provider = %cfg.provider,
                    "bot_challenge: provider configured but secret_key is empty — \
                     falling back to disabled. Set COTERIE__BOT_CHALLENGE__SECRET_KEY.",
                );
                Arc::new(DisabledVerifier)
            } else {
                Arc::new(TurnstileVerifier::new(client, cfg))
            }
        }
        "disabled" => Arc::new(DisabledVerifier),
        other => {
            tracing::warn!(
                provider = %other,
                "bot_challenge: unknown provider — falling back to disabled.",
            );
            Arc::new(DisabledVerifier)
        }
    }
}

#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils {
    use super::*;
    use std::sync::Mutex;

    /// Verifier whose verdict is decided by the supplied closure. Lets
    /// integration tests assert "this token passes, anything else fails"
    /// without hitting a real provider. Also counts calls so tests can
    /// assert ordering vs. rate-limiting (the verifier should NOT be
    /// invoked on rate-limited requests).
    pub struct FakeVerifier {
        decide: Box<dyn Fn(&str) -> Result<(), VerifyError> + Send + Sync>,
        calls: Mutex<u32>,
    }

    impl FakeVerifier {
        pub fn new<F>(decide: F) -> Self
        where
            F: Fn(&str) -> Result<(), VerifyError> + Send + Sync + 'static,
        {
            Self { decide: Box::new(decide), calls: Mutex::new(0) }
        }

        pub fn call_count(&self) -> u32 {
            *self.calls.lock().unwrap()
        }
    }

    #[async_trait]
    impl BotChallengeVerifier for FakeVerifier {
        async fn verify(
            &self,
            _route: &'static str,
            token: Option<&str>,
            _client_ip: Option<IpAddr>,
        ) -> Result<(), VerifyError> {
            *self.calls.lock().unwrap() += 1;
            match token.filter(|s| !s.is_empty()) {
                Some(t) => (self.decide)(t),
                None => Err(VerifyError::Missing),
            }
        }
    }
}
