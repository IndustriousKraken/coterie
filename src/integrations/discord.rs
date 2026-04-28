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
    repository::MemberRepository,
    service::settings_service::{DbDiscordConfig, SettingsService},
};

/// Summary returned by `reconcile_all`. Used to render an admin-facing
/// confirmation message and to log the daily sweep.
#[derive(Debug, Default, Clone)]
pub struct ReconcileSummary {
    /// Members evaluated (had a discord_id and a syncable status).
    pub processed: usize,
    /// Members skipped because their snowflake didn't validate.
    pub skipped_invalid_id: usize,
    /// Members skipped because their status isn't reconciled (Pending).
    pub skipped_pending: usize,
}

/// Whether `s` looks like a Discord snowflake (user / guild / role /
/// channel ID). Snowflakes are 17–20 ASCII digits as of 2026 and are
/// expected to keep growing slowly — accept up to 20 to give us a
/// year or two of headroom before this validator needs revisiting.
pub fn is_valid_snowflake(s: &str) -> bool {
    let len = s.len();
    (17..=20).contains(&len) && s.chars().all(|c| c.is_ascii_digit())
}

/// Build the announcement preview shown in the Discord post.
///
/// Prefers the first paragraph (text up to a blank line) when it's a
/// reasonable length; falls back to a char-boundary-safe ~280-char
/// truncate otherwise. The naive `&s[..280]` would panic on content
/// with a multi-byte UTF-8 character crossing the boundary (any emoji
/// or non-ASCII script will hit this).
fn build_announcement_preview(content: &str) -> String {
    const MAX_CHARS: usize = 280;
    const PARAGRAPH_BUDGET: usize = 500;  // bytes — paragraphs longer than this fall through to char truncate

    let trimmed = content.trim();

    // Look for a paragraph break. Treat both \n\n and \r\n\r\n as a
    // separator — Coterie content can come from either Unix or Windows
    // editors via the admin form.
    let first_paragraph = trimmed
        .split_once("\n\n")
        .or_else(|| trimmed.split_once("\r\n\r\n"))
        .map(|(first, _)| first)
        .unwrap_or(trimmed);

    if first_paragraph.len() <= PARAGRAPH_BUDGET && !first_paragraph.is_empty() {
        // First paragraph is short enough to show in full. Add an
        // ellipsis only if there was more content after it.
        if first_paragraph.len() < trimmed.len() {
            format!("{}…", first_paragraph)
        } else {
            first_paragraph.to_string()
        }
    } else {
        // Char-boundary-safe truncation. Take MAX_CHARS chars regardless
        // of byte width.
        let mut out: String = trimmed.chars().take(MAX_CHARS).collect();
        if out.chars().count() < trimmed.chars().count() {
            out.push('…');
        }
        out
    }
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

    /// Walk every member with a discord_id and re-apply roles from
    /// scratch based on their current status. Idempotent — Discord's
    /// PUT-role endpoint is fine with re-adding a role they already
    /// have, and remove returns 404 (treated as success) if it's
    /// already gone.
    ///
    /// Caller is responsible for not running this concurrently with
    /// itself; a single 500-member club takes ~few seconds at the
    /// rate Discord allows. Failures per-member are logged and don't
    /// abort the rest of the sweep.
    pub async fn reconcile_all(&self, members: Arc<dyn MemberRepository>) -> ReconcileSummary {
        let mut summary = ReconcileSummary::default();
        if self.load().await.is_none() {
            return summary;
        }
        let all = match members.list_with_discord_id().await {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("Discord reconcile: couldn't list members: {}", e);
                return summary;
            }
        };
        for m in &all {
            // Validate before counting as "processed" — we want the
            // summary to reflect actual sync attempts.
            let Some(id) = &m.discord_id else { continue };
            if !is_valid_snowflake(id) {
                summary.skipped_invalid_id += 1;
                continue;
            }
            if matches!(m.status, MemberStatus::Pending) {
                summary.skipped_pending += 1;
                continue;
            }
            self.sync_roles(m).await;
            summary.processed += 1;
        }
        summary
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
                // link drives them to the portal for the rest. Prefer
                // the first paragraph (split on a blank line) if it's
                // a reasonable length, otherwise truncate at ~280 chars
                // on a char boundary — a raw byte slice would panic on
                // non-ASCII content crossing the boundary.
                let preview = build_announcement_preview(&announcement.content);
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

#[cfg(test)]
mod preview_tests {
    use super::build_announcement_preview;

    #[test]
    fn short_single_paragraph_returned_verbatim() {
        let s = build_announcement_preview("We're meeting Saturday at 7pm.");
        assert_eq!(s, "We're meeting Saturday at 7pm.");
    }

    #[test]
    fn first_paragraph_with_more_content_gets_ellipsis() {
        let s = build_announcement_preview(
            "Quick note about Saturday.\n\nLong details follow about the event ..."
        );
        assert_eq!(s, "Quick note about Saturday.…");
    }

    #[test]
    fn long_first_paragraph_falls_back_to_char_truncate() {
        // Single paragraph longer than PARAGRAPH_BUDGET (500 bytes) → char-truncate to 280
        let long = "x".repeat(600);
        let s = build_announcement_preview(&long);
        assert_eq!(s.chars().filter(|c| *c == 'x').count(), 280);
        assert!(s.ends_with('…'));
    }

    #[test]
    fn does_not_panic_on_multibyte_chars_at_boundary() {
        // 281 emojis = ~1124 bytes (each emoji is 4 bytes UTF-8). Naive
        // byte-slicing at index 280 would panic; char-aware truncation
        // works fine.
        let emoji_wall = "🎉".repeat(300);
        let s = build_announcement_preview(&emoji_wall);
        assert_eq!(s.chars().filter(|c| *c == '🎉').count(), 280);
        assert!(s.ends_with('…'));
    }

    #[test]
    fn handles_crlf_paragraph_separator() {
        let s = build_announcement_preview("Hi.\r\n\r\nDetails ...");
        assert_eq!(s, "Hi.…");
    }

    #[test]
    fn empty_first_paragraph_falls_through() {
        // Leading blank lines shouldn't produce an empty preview.
        let s = build_announcement_preview("Real content");
        assert_eq!(s, "Real content");
    }
}
