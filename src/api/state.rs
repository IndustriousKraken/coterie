use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::http::HeaderMap;

use crate::{
    config::Settings,
    payments::StripeClient,
    service::ServiceContext,
};

/// Extract client IP from request headers, checking reverse-proxy headers first.
pub fn client_ip(headers: &HeaderMap) -> IpAddr {
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
    // Fallback — localhost (behind proxy, this is the best we can do without ConnectInfo)
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
        let mut map = self.attempts.lock().unwrap();
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
        let mut map = self.attempts.lock().unwrap();
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
    pub settings: Arc<Settings>,
    /// Rate limiter for login endpoints (5 attempts per 15 minutes per IP).
    pub login_limiter: RateLimiter,
}

impl AppState {
    pub fn new(
        service_context: Arc<ServiceContext>,
        stripe_client: Option<Arc<StripeClient>>,
        settings: Arc<Settings>,
    ) -> Self {
        Self {
            service_context,
            stripe_client,
            settings,
            login_limiter: RateLimiter::new(5, Duration::from_secs(15 * 60)),
        }
    }
}