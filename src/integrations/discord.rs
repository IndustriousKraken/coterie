//! Discord integration. Reads its config from the DB on every event
//! (matching the email DynamicSender pattern) so admin edits take
//! effect without restart. Skips gracefully when:
//!
//!   - The integration is disabled
//!   - The member has no `discord_id` (we don't know who they are
//!     on Discord; nothing to sync)
//!   - Required role IDs aren't configured
//!
//! Failures are logged but never bubble up to the caller — a Discord
//! outage shouldn't fail an admin's "suspend member" action.

use async_trait::async_trait;
use std::sync::Arc;

use crate::{
    domain::{Member, MemberStatus},
    error::Result,
    integrations::{
        Integration, IntegrationEvent,
        discord_client::DiscordClient,
    },
    service::settings_service::{DbDiscordConfig, SettingsService},
};

/// Whether `s` looks like a Discord snowflake (user / guild / role /
/// channel ID). Snowflakes are 17–20 ASCII digits as of 2026 and are
/// expected to keep growing slowly — accept up to 20 to give us a
/// year or two of headroom before this validator needs revisiting.
pub fn is_valid_snowflake(s: &str) -> bool {
    let len = s.len();
    (17..=20).contains(&len) && s.chars().all(|c| c.is_ascii_digit())
}

pub struct DiscordIntegration {
    settings: Arc<SettingsService>,
    /// Absolute Coterie base URL (from ServerConfig::base_url), used
    /// to build links in outgoing Discord messages so members can
    /// click through to events/announcements/payment pages.
    base_url: String,
}

impl DiscordIntegration {
    pub fn new(settings: Arc<SettingsService>, base_url: String) -> Self {
        Self { settings, base_url }
    }

    /// Pull the live config + a ready-to-use HTTP client. Returns
    /// `None` if the integration is disabled or the bot token is
    /// empty/missing — in either case there's nothing to do.
    async fn load(&self) -> Option<(DbDiscordConfig, DiscordClient)> {
        let cfg = match self.settings.get_discord_config().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Discord integration: couldn't load config: {}", e);
                return None;
            }
        };
        if !cfg.enabled || cfg.bot_token.is_empty() || cfg.guild_id.is_empty() {
            return None;
        }
        let client = DiscordClient::new(cfg.bot_token.clone());
        Some((cfg, client))
    }

    /// Apply roles for a member's CURRENT status. Logs and returns Ok
    /// for missing-role / missing-discord-id cases — skipping them is
    /// a feature, not a bug, and we don't want the admin's primary
    /// action to error because of an integration gap.
    async fn sync_roles(&self, member: &Member) {
        let Some((cfg, client)) = self.load().await else {
            return;
        };
        let Some(discord_id) = &member.discord_id else {
            tracing::debug!(
                "Discord sync skipped for member {}: no discord_id on file",
                member.id
            );
            return;
        };
        if !is_valid_snowflake(discord_id) {
            tracing::warn!(
                "Discord sync skipped for member {}: invalid discord_id {:?}",
                member.id, discord_id
            );
            return;
        }

        match member.status {
            MemberStatus::Active | MemberStatus::Honorary => {
                if !cfg.member_role_id.is_empty() {
                    if let Err(e) = client.add_role(&cfg.guild_id, discord_id, &cfg.member_role_id).await {
                        tracing::error!("Discord add member role for {}: {}", member.id, e);
                    }
                }
                if !cfg.expired_role_id.is_empty() {
                    if let Err(e) = client.remove_role(&cfg.guild_id, discord_id, &cfg.expired_role_id).await {
                        tracing::error!("Discord remove expired role for {}: {}", member.id, e);
                    }
                }
            }
            MemberStatus::Expired | MemberStatus::Suspended => {
                if !cfg.expired_role_id.is_empty() {
                    if let Err(e) = client.add_role(&cfg.guild_id, discord_id, &cfg.expired_role_id).await {
                        tracing::error!("Discord add expired role for {}: {}", member.id, e);
                    }
                }
                if !cfg.member_role_id.is_empty() {
                    if let Err(e) = client.remove_role(&cfg.guild_id, discord_id, &cfg.member_role_id).await {
                        tracing::error!("Discord remove member role for {}: {}", member.id, e);
                    }
                }
            }
            MemberStatus::Pending => {
                // Hasn't been approved yet — they shouldn't have ANY
                // Coterie-owned role. Strip both. They typically aren't
                // in the guild at all at this stage so the calls 404
                // quietly.
                if !cfg.member_role_id.is_empty() {
                    let _ = client.remove_role(&cfg.guild_id, discord_id, &cfg.member_role_id).await;
                }
                if !cfg.expired_role_id.is_empty() {
                    let _ = client.remove_role(&cfg.guild_id, discord_id, &cfg.expired_role_id).await;
                }
            }
        }
    }

    /// Post a message to a configured channel. No-op (with a debug
    /// trace) when the channel ID is empty — the operator just hasn't
    /// set up that channel.
    async fn post_to_channel(&self, channel_id: &str, content: &str) {
        let Some((_, client)) = self.load().await else {
            return;
        };
        if channel_id.is_empty() {
            return;
        }
        if let Err(e) = client.send_message(channel_id, content).await {
            tracing::error!(
                "Discord send_message to channel {}: {}",
                channel_id, e
            );
        }
    }

    /// Strip any Coterie-managed role. Used on member deletion.
    async fn clear_roles(&self, member: &Member) {
        let Some((cfg, client)) = self.load().await else {
            return;
        };
        let Some(discord_id) = &member.discord_id else {
            return;
        };
        if !is_valid_snowflake(discord_id) {
            return;
        }
        if !cfg.member_role_id.is_empty() {
            let _ = client.remove_role(&cfg.guild_id, discord_id, &cfg.member_role_id).await;
        }
        if !cfg.expired_role_id.is_empty() {
            let _ = client.remove_role(&cfg.guild_id, discord_id, &cfg.expired_role_id).await;
        }
    }
}

#[async_trait]
impl Integration for DiscordIntegration {
    fn name(&self) -> &str {
        "Discord"
    }

    fn is_enabled(&self) -> bool {
        // Always "registered" — we re-check enable/configured state
        // on every event, since the DB is the source of truth and an
        // admin can flip it at any time.
        true
    }

    async fn health_check(&self) -> Result<()> {
        // Best-effort: if disabled or unconfigured, that's not an
        // error — it's "intentionally off." Only signal an error if
        // the integration is supposed to be working but can't reach
        // Discord.
        let Some((_, client)) = self.load().await else {
            return Ok(());
        };
        match client.get_current_user().await {
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        }
    }

    async fn handle_event(&self, event: &IntegrationEvent) -> Result<()> {
        match event {
            IntegrationEvent::MemberCreated(_) => {
                // Pending member; nothing to do until they're activated.
                Ok(())
            }
            IntegrationEvent::MemberActivated(m) => {
                self.sync_roles(m).await;
                Ok(())
            }
            IntegrationEvent::MemberExpired(m) => {
                self.sync_roles(m).await;
                Ok(())
            }
            IntegrationEvent::MemberDeleted(m) => {
                self.clear_roles(m).await;
                Ok(())
            }
            IntegrationEvent::MemberUpdated { old, new } => {
                // Two reasons we'd need to act:
                //   1. Status changed → roles need to follow
                //   2. discord_id changed → strip old, apply new
                let status_changed = old.status != new.status;
                let id_changed = old.discord_id != new.discord_id;
                if id_changed {
                    // Make sure roles aren't lingering on the old user.
                    if old.discord_id.is_some() {
                        self.clear_roles(old).await;
                    }
                }
                if status_changed || id_changed {
                    self.sync_roles(new).await;
                }
                Ok(())
            }

            IntegrationEvent::EventPublished(event) => {
                let Some((cfg, _)) = self.load().await else {
                    return Ok(());
                };
                // AdminOnly events go to the admin alerts channel
                // (members shouldn't see those), public/members-only
                // go to the events channel.
                let channel = match event.visibility {
                    crate::domain::EventVisibility::AdminOnly => &cfg.admin_alerts_channel_id,
                    _ => &cfg.events_channel_id,
                };
                if channel.is_empty() {
                    return Ok(());
                }
                let prefix = match event.visibility {
                    crate::domain::EventVisibility::AdminOnly => "**[Admin only]** ",
                    crate::domain::EventVisibility::MembersOnly => "**[Members only]** ",
                    _ => "",
                };
                let when = event.start_time.format("%a %b %d, %Y at %H:%M UTC");
                let location = event.location.as_deref().unwrap_or("(no location set)");
                let link = format!(
                    "{}/portal/events/{}",
                    self.base_url.trim_end_matches('/'),
                    event.id
                );
                let content = format!(
                    "{}📅 **New event: {}**\n{}\nWhere: {}\nDetails: {}",
                    prefix, event.title, when, location, link,
                );
                self.post_to_channel(channel, &content).await;
                Ok(())
            }

            IntegrationEvent::AnnouncementPublished(announcement) => {
                let Some((cfg, _)) = self.load().await else {
                    return Ok(());
                };
                if cfg.announcements_channel_id.is_empty() {
                    return Ok(());
                }
                let visibility_tag = if announcement.is_public {
                    ""
                } else {
                    "**[Members only]** "
                };
                // Trim the body for chat: full content can be long, the
                // link drives them to the portal for the rest.
                let preview = if announcement.content.len() > 280 {
                    format!("{}…", &announcement.content[..280])
                } else {
                    announcement.content.clone()
                };
                let link = format!(
                    "{}/portal/announcements",
                    self.base_url.trim_end_matches('/'),
                );
                let content = format!(
                    "{}📣 **{}**\n{}\n\n{}",
                    visibility_tag, announcement.title, preview, link,
                );
                self.post_to_channel(&cfg.announcements_channel_id, &content).await;
                Ok(())
            }

            IntegrationEvent::AdminAlert { subject, body } => {
                let Some((cfg, _)) = self.load().await else {
                    return Ok(());
                };
                if cfg.admin_alerts_channel_id.is_empty() {
                    return Ok(());
                }
                let content = format!("⚠️ **{}**\n{}", subject, body);
                self.post_to_channel(&cfg.admin_alerts_channel_id, &content).await;
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod snowflake_tests {
    use super::*;

    #[test]
    fn accepts_valid() {
        assert!(is_valid_snowflake("123456789012345678")); // 18 digits
        assert!(is_valid_snowflake("12345678901234567"));   // 17 digits
        assert!(is_valid_snowflake("12345678901234567890")); // 20 digits
    }

    #[test]
    fn rejects_invalid() {
        assert!(!is_valid_snowflake(""));
        assert!(!is_valid_snowflake("123"));                 // too short
        assert!(!is_valid_snowflake("123456789012345678901")); // too long (21)
        assert!(!is_valid_snowflake("user#1234"));           // legacy username format
        assert!(!is_valid_snowflake("12345678901234567a"));  // non-digit
        assert!(!is_valid_snowflake(" 123456789012345678")); // leading space
    }
}
