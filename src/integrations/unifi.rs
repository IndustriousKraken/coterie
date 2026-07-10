//! UniFi Access integration. Reads its config from the DB on every event
//! (matching the Discord pattern) so admin edits take effect without a
//! restart. Skips gracefully when the integration is disabled or has no
//! controller URL. The door-control calls are still stubs — the real work
//! here is that config is DB-backed and read at operation time.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::{
    domain::MemberStatus,
    error::Result,
    integrations::{Integration, IntegrationEvent},
    service::settings_service::{DbUnifiConfig, SettingsService},
};

pub struct UnifiIntegration {
    settings: Arc<SettingsService>,
}

impl UnifiIntegration {
    pub fn new(settings: Arc<SettingsService>) -> Self {
        Self { settings }
    }

    /// Pull the live config from the DB. Returns `None` if the integration
    /// is disabled or has no controller URL — in either case there's
    /// nothing to do. A decrypt failure surfaces as "unconfigured".
    async fn load(&self) -> Option<DbUnifiConfig> {
        let cfg = match self.settings.get_unifi_config().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Unifi integration: couldn't load config: {}", e);
                return None;
            }
        };
        if !cfg.enabled || cfg.controller_url.is_empty() {
            return None;
        }
        Some(cfg)
    }

    async fn grant_access(&self, member_email: &str) -> Result<()> {
        // Implementation would:
        // 1. Create user in Unifi Access if not exists
        // 2. Assign access groups
        // 3. Sync to door controllers
        tracing::info!("Would grant Unifi access to: {}", member_email);
        Ok(())
    }

    async fn revoke_access(&self, member_email: &str) -> Result<()> {
        // Implementation would:
        // 1. Find user in Unifi system
        // 2. Remove from access groups
        // 3. Optionally delete user
        tracing::info!("Would revoke Unifi access from: {}", member_email);
        Ok(())
    }

    async fn update_access(&self, member_email: &str, active: bool) -> Result<()> {
        if active {
            self.grant_access(member_email).await
        } else {
            self.revoke_access(member_email).await
        }
    }
}

#[async_trait]
impl Integration for UnifiIntegration {
    fn name(&self) -> &str {
        "Unifi"
    }

    fn is_enabled(&self) -> bool {
        // Always "registered" — we re-check enable/configured state on
        // every event, since the DB is the source of truth and an admin
        // can flip it at any time (same pattern as Discord).
        true
    }

    async fn health_check(&self) -> Result<()> {
        // Best-effort: disabled/unconfigured is "intentionally off," not an
        // error. We don't ping the controller here — that's what the admin
        // "Test connection" button is for.
        let _ = self.load().await;
        Ok(())
    }

    async fn handle_event(&self, event: &IntegrationEvent) -> Result<()> {
        // Gate on the live DB config: nothing to do if disabled/unconfigured.
        if self.load().await.is_none() {
            return Ok(());
        }
        match event {
            IntegrationEvent::MemberActivated(member) => {
                self.grant_access(&member.email).await?;
            }
            IntegrationEvent::MemberExpired(member) => {
                self.revoke_access(&member.email).await?;
            }
            IntegrationEvent::MemberUpdated { old: _, new } => {
                // Update access based on new status
                let should_have_access =
                    matches!(new.status, MemberStatus::Active | MemberStatus::Honorary);
                self.update_access(&new.email, should_have_access).await?;
            }
            _ => {}
        }
        Ok(())
    }
}

/// Authenticate to a UniFi controller with the given credentials and report
/// (ok, human-readable detail). Used by the admin "Test connection" button;
/// it never persists anything. Tries the UniFi OS login path first, then
/// the legacy UniFi Network path. Accepts self-signed certs — UniFi
/// controllers ship with them and admins test against their own hardware.
///
// ponytail: connectivity + credential probe only — no session reuse, CSRF
// token handling, or 2FA. Upgrade to a real UnifiClient when the
// door-control calls (grant/revoke) stop being stubs.
pub async fn test_connection(
    controller_url: &str,
    username: &str,
    password: &str,
) -> (bool, String) {
    let base = controller_url.trim().trim_end_matches('/');
    if base.is_empty() {
        return (false, "No controller URL configured.".to_string());
    }
    if username.is_empty() || password.is_empty() {
        return (
            false,
            "Username and password are required to test the connection.".to_string(),
        );
    }

    let client = match reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => return (false, format!("Couldn't build HTTP client: {}", e)),
    };

    let body = serde_json::json!({ "username": username, "password": password });

    // UniFi OS consoles use /api/auth/login; the legacy self-hosted UniFi
    // Network application uses /api/login. Try the modern path first and
    // fall back on a 404 (wrong generation for this controller).
    for path in ["/api/auth/login", "/api/login"] {
        let url = format!("{}{}", base, path);
        match client.post(&url).json(&body).send().await {
            Ok(resp) => {
                let code = resp.status().as_u16();
                if resp.status().is_success() {
                    return (
                        true,
                        format!("Authenticated to the UniFi controller ({}).", path),
                    );
                }
                // 400/401 = reached the controller, credentials rejected —
                // a definitive answer; don't bother trying the other path.
                if code == 400 || code == 401 {
                    return (
                        false,
                        format!("Controller rejected the credentials (HTTP {}).", code),
                    );
                }
                // 404 (or anything else) → likely the wrong login path for
                // this controller generation; fall through and try the next.
            }
            Err(e) => {
                // Connect/timeout won't improve on the other path — report now.
                if e.is_connect() || e.is_timeout() {
                    return (false, format!("Couldn't reach the controller: {}", e));
                }
            }
        }
    }

    (
        false,
        "Couldn't authenticate — no known UniFi login endpoint responded.".to_string(),
    )
}
