use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::http::HeaderMap;
use tokio::sync::Mutex as AsyncMutex;

use crate::{
    config::Settings,
    payments::{StripeClient, WebhookDispatcher},
    service::ServiceContext,
};

/// Extract client IP from request headers.
///
/// If `trust_forwarded` is false, X-Forwarded-For / X-Real-Ip are ignored
/// entirely (they can be spoofed by any client) and the fallback is used.
/// Set this based on whether the server sits behind a trusted reverse
/// proxy — see `ServerConfig::trust_forwarded_for`.
pub fn client_ip(headers: &HeaderMap, trust_forwarded: bool) -> IpAddr {
    if trust_forwarded {
        // Try X-Forwarded-For (first IP in the chain is the client)
        if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
            if let Some(first) = xff.split(',').next() {
                if let Ok(ip) = first.trim().parse::<IpAddr>() {
                    return ip;
                }
            }
        }
        // Try X-Real-Ip
        if let Some(xri) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
            if let Ok(ip) = xri.trim().parse::<IpAddr>() {
                return ip;
            }
        }
    }
    // Fallback: localhost. We don't use ConnectInfo at the moment; when
    // `trust_forwarded` is false and the peer IP is unavailable, rate
    // limiting collapses to a single bucket. Safer than trusting a
    // client-supplied header.
    IpAddr::from([127, 0, 0, 1])
}

/// Simple in-memory rate limiter keyed by IP address.
#[derive(Clone)]
pub struct RateLimiter {
    /// Map of IP -> list of attempt timestamps within the window.
    attempts: Arc<Mutex<HashMap<IpAddr, Vec<Instant>>>>,
    /// Maximum attempts allowed within `window`.
    max_attempts: usize,
    /// Sliding window duration.
    window: Duration,
}

impl RateLimiter {
    pub fn new(max_attempts: usize, window: Duration) -> Self {
        Self {
            attempts: Arc::new(Mutex::new(HashMap::new())),
            max_attempts,
            window,
        }
    }

    /// Returns `true` if the request is allowed, `false` if rate-limited.
    /// Automatically records the attempt when allowed.
    pub fn check_and_record(&self, ip: IpAddr) -> bool {
        // Recover from a poisoned mutex rather than propagating the
        // panic. A poisoned state means some prior call panicked while
        // holding the lock — the data may be slightly stale but the
        // rate limiter is best-effort anyway, and falling over here
        // would deny service to every login attempt.
        let mut map = match self.attempts.lock() {
            Ok(g) => g,
            Err(poisoned) => {
                tracing::warn!("RateLimiter mutex was poisoned; recovering");
                poisoned.into_inner()
            }
        };
        let now = Instant::now();
        let cutoff = now - self.window;

        let timestamps = map.entry(ip).or_default();
        timestamps.retain(|t| *t > cutoff);

        if timestamps.len() >= self.max_attempts {
            return false;
        }

        timestamps.push(now);
        true
    }

    /// Prune entries for IPs that have no recent attempts. Call periodically
    /// to prevent the map from growing unboundedly.
    pub fn cleanup(&self) {
        let mut map = match self.attempts.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let cutoff = Instant::now() - self.window;
        map.retain(|_, timestamps| {
            timestamps.retain(|t| *t > cutoff);
            !timestamps.is_empty()
        });
    }
}

#[derive(Clone)]
pub struct AppState {
    pub service_context: Arc<ServiceContext>,
    pub stripe_client: Option<Arc<StripeClient>>,
    /// Inbound webhook dispatcher. `Some` exactly when `stripe_client`
    /// is `Some` (both depend on Stripe being configured).
    pub webhook_dispatcher: Option<Arc<WebhookDispatcher>>,
    pub settings: Arc<Settings>,
    /// Rate limiter for login endpoints (5 attempts per 15 minutes per IP).
    pub login_limiter: RateLimiter,
    /// Rate limiter for money-moving endpoints (charge, donate, refund,
    /// auto-renew toggle). 10 attempts/min per IP — well above any
    /// legitimate workflow but tight enough to box in scripted abuse,
    /// double-submit accidents, and runaway clients. Per-IP rather
    /// than per-member because the source of an attack is the network,
    /// not the authenticated identity (which an attacker controlling
    /// a stolen session would also control).
    pub money_limiter: RateLimiter,
    /// Serializes first-admin setup to prevent concurrent requests from
    /// both passing the "no admin exists" check and creating two admins.
    pub setup_lock: Arc<AsyncMutex<()>>,
}

impl AppState {
    pub fn new(
        service_context: Arc<ServiceContext>,
        stripe_client: Option<Arc<StripeClient>>,
        webhook_dispatcher: Option<Arc<WebhookDispatcher>>,
        settings: Arc<Settings>,
    ) -> Self {
        Self {
            service_context,
            stripe_client,
            webhook_dispatcher,
            settings,
            login_limiter: RateLimiter::new(5, Duration::from_secs(15 * 60)),
            money_limiter: RateLimiter::new(10, Duration::from_secs(60)),
            setup_lock: Arc::new(AsyncMutex::new(())),
        }
    }
}