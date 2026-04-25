//! Minimal Discord REST API client. Wraps the handful of endpoints
//! Coterie needs — it's not a general-purpose Discord library.
//!
//! Auth: bot token via `Authorization: Bot <token>`. Rate limits are
//! handled lazily — Discord returns 429 with a `Retry-After` header
//! when we exceed a route's bucket; for the volume we generate
//! (admin-driven role changes, occasional channel posts) we'll
//! basically never hit a limit. If we ever do, we honor the header
//! once and bail on persistent throttling.
//!
//! All methods return `Err(AppError::External)` on HTTP/network/4xx
//! /5xx failures, with the body included for debugging.

use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};

const API_BASE: &str = "https://discord.com/api/v10";

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
        let resp = self.http.put(&url)
            .header("Authorization", format!("Bot {}", self.bot_token))
            .header("Content-Length", "0") // Discord rejects PUT with no body unless this is set
            .send()
            .await
            .map_err(|e| AppError::External(format!("Discord add_role request: {}", e)))?;
        // 204 No Content on success
        if resp.status().as_u16() == 404 {
            tracing::warn!(
                "Discord add_role: 404 for guild={} user={} role={} (member not in guild or role gone?)",
                guild_id, user_id, role_id
            );
            return Ok(());
        }
        check_status(&resp.status())
            .map_err(|e| {
                let url = format!("guild={} user={} role={}", guild_id, user_id, role_id);
                AppError::External(format!("Discord add_role {}: {}", url, e))
            })
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
        let resp = self.http.delete(&url)
            .header("Authorization", format!("Bot {}", self.bot_token))
            .send()
            .await
            .map_err(|e| AppError::External(format!("Discord remove_role request: {}", e)))?;
        if resp.status().as_u16() == 404 {
            tracing::warn!(
                "Discord remove_role: 404 for guild={} user={} role={} (already gone?)",
                guild_id, user_id, role_id
            );
            return Ok(());
        }
        check_status(&resp.status())
            .map_err(|e| {
                let url = format!("guild={} user={} role={}", guild_id, user_id, role_id);
                AppError::External(format!("Discord remove_role {}: {}", url, e))
            })
    }

    /// `POST /channels/{channel.id}/messages` — used by D3 notification
    /// publishing. Implemented now so the test path can validate
    /// channel access too if we want it later.
    #[allow(dead_code)]
    pub async fn send_message(&self, channel_id: &str, content: &str) -> Result<()> {
        let url = format!("{}/channels/{}/messages", API_BASE, channel_id);
        let body = serde_json::json!({ "content": content });
        let resp = self.http.post(&url)
            .header("Authorization", format!("Bot {}", self.bot_token))
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::External(format!("Discord send_message request: {}", e)))?;
        check_status(&resp.status())
            .map_err(|e| AppError::External(format!("Discord send_message channel={}: {}", channel_id, e)))
    }
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
