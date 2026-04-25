//! Minimal Discord REST API client. Wraps the handful of endpoints
//! Coterie needs — it's not a general-purpose Discord library.
//!
//! Auth: bot token via `Authorization: Bot <token>`. Rate limits and
//! transient failures are retried in-process: up to 3 attempts with
//! exponential backoff for connection/timeout errors and 5xx, and a
//! bounded honor-the-header wait for 429s. The connection-test path
//! (`get_current_user`) intentionally bypasses retries — admins want
//! immediate feedback when they click "Test connection."
//!
//! All methods return `Err(AppError::External)` on HTTP/network/4xx
//! /5xx failures, with the body included for debugging.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};

const API_BASE: &str = "https://discord.com/api/v10";
const MAX_ATTEMPTS: usize = 3;
/// Cap on how long we'll honor a `Retry-After` header. Discord rarely
/// asks for more than a second or two; if it asks for a minute we'd
/// rather give up and let the reconcile sweep clean up later than block
/// an admin action.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(5);

/// Subset of Discord's User object — just the bits we use for the
/// connection-test response.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DiscordUser {
    pub id: String,
    pub username: String,
    /// Some bots expose a discriminator (legacy 4-digit suffix).
    /// Optional; new "next-gen" usernames don't have one.
    #[serde(default)]
    pub discriminator: Option<String>,
}

pub struct DiscordClient {
    http: reqwest::Client,
    bot_token: String,
}

impl DiscordClient {
    /// Build a client. `bot_token` is the raw token from Discord's
    /// developer portal — we'll prepend "Bot " ourselves on each
    /// request.
    pub fn new(bot_token: String) -> Self {
        // The User-Agent is REQUIRED by Discord's API docs. They use
        // it for abuse tracking; sending a generic reqwest UA has been
        // known to hit weird rate limits.
        let http = reqwest::Client::builder()
            .user_agent("Coterie (https://github.com/IndustriousKraken/coterie, 0.1)")
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { http, bot_token }
    }

    /// `GET /users/@me` — used for the admin "test connection" button.
    /// Returns the bot's own user object on success.
    ///
    /// No retry: this is the test-connection path, and an admin
    /// staring at a spinner wants the answer as fast as possible.
    pub async fn get_current_user(&self) -> Result<DiscordUser> {
        let url = format!("{}/users/@me", API_BASE);
        let resp = self.http.get(&url)
            .header("Authorization", format!("Bot {}", self.bot_token))
            .send()
            .await
            .map_err(|e| AppError::External(format!("Discord request failed: {}", e)))?;
        check_status(&resp.status())?;
        let body = resp.text().await
            .map_err(|e| AppError::External(format!("Discord response read failed: {}", e)))?;
        serde_json::from_str(&body)
            .map_err(|e| AppError::External(format!("Discord response parse: {} (body: {})", e, body)))
    }

    /// `PUT /guilds/{guild.id}/members/{user.id}/roles/{role.id}` —
    /// gives a guild member a role. Idempotent; calling twice is fine.
    /// Returns Ok on 204 (success) and on 404 ("user isn't in this
    /// guild" or "role doesn't exist") since neither is something we
    /// can fix from here, just log and move on.
    pub async fn add_role(
        &self,
        guild_id: &str,
        user_id: &str,
        role_id: &str,
    ) -> Result<()> {
        let url = format!(
            "{}/guilds/{}/members/{}/roles/{}",
            API_BASE, guild_id, user_id, role_id
        );
        let label = format!("add_role guild={} user={} role={}", guild_id, user_id, role_id);
        let resp = send_with_retry(&label, || {
            self.http.put(&url)
                .header("Authorization", format!("Bot {}", self.bot_token))
                .header("Content-Length", "0") // Discord rejects PUT with no body unless this is set
        }).await?;
        if resp.status().as_u16() == 404 {
            tracing::warn!(
                "Discord add_role: 404 for guild={} user={} role={} (member not in guild or role gone?)",
                guild_id, user_id, role_id
            );
            return Ok(());
        }
        check_status(&resp.status())
            .map_err(|e| AppError::External(format!("Discord {}: {}", label, e)))
    }

    /// `DELETE /guilds/{guild.id}/members/{user.id}/roles/{role.id}` —
    /// removes a role. Idempotent; 404 is treated as success (already gone).
    pub async fn remove_role(
        &self,
        guild_id: &str,
        user_id: &str,
        role_id: &str,
    ) -> Result<()> {
        let url = format!(
            "{}/guilds/{}/members/{}/roles/{}",
            API_BASE, guild_id, user_id, role_id
        );
        let label = format!("remove_role guild={} user={} role={}", guild_id, user_id, role_id);
        let resp = send_with_retry(&label, || {
            self.http.delete(&url)
                .header("Authorization", format!("Bot {}", self.bot_token))
        }).await?;
        if resp.status().as_u16() == 404 {
            tracing::warn!(
                "Discord remove_role: 404 for guild={} user={} role={} (already gone?)",
                guild_id, user_id, role_id
            );
            return Ok(());
        }
        check_status(&resp.status())
            .map_err(|e| AppError::External(format!("Discord {}: {}", label, e)))
    }

    /// `POST /channels/{channel.id}/messages` — used by D3 notification
    /// publishing.
    pub async fn send_message(&self, channel_id: &str, content: &str) -> Result<()> {
        let url = format!("{}/channels/{}/messages", API_BASE, channel_id);
        let body = serde_json::json!({ "content": content });
        let label = format!("send_message channel={}", channel_id);
        let resp = send_with_retry(&label, || {
            self.http.post(&url)
                .header("Authorization", format!("Bot {}", self.bot_token))
                .json(&body)
        }).await?;
        check_status(&resp.status())
            .map_err(|e| AppError::External(format!("Discord {}: {}", label, e)))
    }
}

/// Drive a request through up to MAX_ATTEMPTS, retrying transient
/// connection errors and 5xx, and honoring `Retry-After` on 429.
///
/// Takes a closure that builds the request rather than a RequestBuilder
/// directly — simpler than `try_clone`, and handles the (rare) case
/// where reqwest can't clone a streaming body.
async fn send_with_retry<F>(label: &str, build: F) -> Result<reqwest::Response>
where
    F: Fn() -> reqwest::RequestBuilder,
{
    let mut last_err: Option<String> = None;
    for attempt in 1..=MAX_ATTEMPTS {
        match build().send().await {
            Ok(resp) => {
                let code = resp.status().as_u16();
                let is_retryable = code == 429 || (500..=599).contains(&code);
                if is_retryable && attempt < MAX_ATTEMPTS {
                    let delay = if code == 429 {
                        retry_after(&resp).unwrap_or_else(|| backoff_delay(attempt))
                    } else {
                        backoff_delay(attempt)
                    };
                    tracing::warn!(
                        "Discord {}: HTTP {} on attempt {}/{}, retrying in {:?}",
                        label, code, attempt, MAX_ATTEMPTS, delay,
                    );
                    tokio::time::sleep(delay).await;
                    continue;
                }
                return Ok(resp);
            }
            Err(e) => {
                let transient = e.is_timeout() || e.is_connect() || e.is_request();
                if transient && attempt < MAX_ATTEMPTS {
                    let delay = backoff_delay(attempt);
                    tracing::warn!(
                        "Discord {}: transient error {} on attempt {}/{}, retrying in {:?}",
                        label, e, attempt, MAX_ATTEMPTS, delay,
                    );
                    last_err = Some(e.to_string());
                    tokio::time::sleep(delay).await;
                    continue;
                }
                return Err(AppError::External(format!("Discord {} request: {}", label, e)));
            }
        }
    }
    // Loop exits via return; this is only reached if every attempt
    // hit a transient error and we exhausted retries — `last_err` is
    // always populated in that case.
    Err(AppError::External(format!(
        "Discord {} request: {}",
        label,
        last_err.unwrap_or_else(|| "exhausted retries".into()),
    )))
}

/// 500ms, 1s, 2s, … exponential.
fn backoff_delay(attempt: usize) -> Duration {
    let exp = (attempt - 1).min(6) as u32;
    Duration::from_millis(500u64.saturating_mul(1u64 << exp))
}

/// Parse the `Retry-After` header (seconds, possibly fractional —
/// Discord uses floats). Capped at MAX_RETRY_AFTER so a misbehaving
/// upstream can't pin us indefinitely.
fn retry_after(resp: &reqwest::Response) -> Option<Duration> {
    let raw = resp.headers().get("retry-after")?.to_str().ok()?;
    let secs: f64 = raw.parse().ok()?;
    if secs.is_nan() || secs < 0.0 {
        return None;
    }
    let dur = Duration::from_millis((secs * 1000.0) as u64);
    Some(dur.min(MAX_RETRY_AFTER))
}

/// Translate an HTTP status into an Ok/Err. Discord returns 2xx (with
/// 204 being typical for role mutations), 401 for bad token, 403 for
/// missing permissions, 429 for rate limit, 5xx for their own outages.
fn check_status(status: &reqwest::StatusCode) -> Result<()> {
    if status.is_success() {
        return Ok(());
    }
    let detail = match status.as_u16() {
        401 => "401 Unauthorized — bot token invalid or revoked",
        403 => "403 Forbidden — bot is missing the required permission, or its role is below the role it's trying to manage",
        404 => "404 Not Found",
        429 => "429 Too Many Requests — rate limited",
        500..=599 => "Discord returned 5xx",
        _ => "request failed",
    };
    Err(AppError::External(format!("{} ({})", detail, status)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_then_caps() {
        assert_eq!(backoff_delay(1), Duration::from_millis(500));
        assert_eq!(backoff_delay(2), Duration::from_millis(1000));
        assert_eq!(backoff_delay(3), Duration::from_millis(2000));
        // Way beyond MAX_ATTEMPTS — just make sure we don't overflow.
        let _ = backoff_delay(20);
    }
}
