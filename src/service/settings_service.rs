use std::sync::Arc;

use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

use std::sync::atomic::{AtomicBool, Ordering};

use crate::{
    auth::SecretCrypto,
    domain::{AppSetting, SettingType, SettingsCategory, UpdateSettingRequest},
    error::{AppError, Result},
};

/// Process-wide cache of the `submissions.enabled` toggle.
///
/// The shared portal layout (`templates/layouts/base.html`) gates the admin
/// "Submissions" nav link on this, so the link appears only when the feature is
/// on. The layout only sees `BaseContext`, and threading the settings service
/// through all `BaseContext::for_member` call sites to read one bool is not
/// worth it — instead this flag is primed from the DB at startup and refreshed
/// whenever `submissions.enabled` is written, mirroring the cached-flag pattern
/// used for `admin_exists_observed`.
static SUBMISSIONS_ENABLED: AtomicBool = AtomicBool::new(false);

/// Current cached value of `submissions.enabled` (default `false`).
pub fn submissions_enabled_cached() -> bool {
    SUBMISSIONS_ENABLED.load(Ordering::Relaxed)
}

/// Set the cached `submissions.enabled` flag. Called at startup (primed from the
/// DB) and on every write to the setting.
pub fn set_submissions_enabled_cached(enabled: bool) {
    SUBMISSIONS_ENABLED.store(enabled, Ordering::Relaxed);
}

/// Keys for organization-wide settings.
pub mod org_keys {
    /// IANA timezone name events are scheduled in (default `UTC`).
    pub const TIMEZONE: &str = "org.timezone";
    /// The org's public account-signup page. Empty (the default) means the
    /// login page advertises no create-account link at all.
    pub const SIGNUP_URL: &str = "org.signup_url";
}

/// Keys used for email configuration. One source of truth so the
/// settings table and handlers can't drift.
pub mod email_keys {
    pub const MODE: &str = "email.mode";
    pub const FROM_ADDRESS: &str = "email.from_address";
    pub const FROM_NAME: &str = "email.from_name";
    pub const SMTP_HOST: &str = "email.smtp_host";
    pub const SMTP_PORT: &str = "email.smtp_port";
    pub const SMTP_USERNAME: &str = "email.smtp_username";
    pub const SMTP_PASSWORD: &str = "email.smtp_password";
    pub const LAST_TEST_AT: &str = "email.last_test_at";
    pub const LAST_TEST_OK: &str = "email.last_test_ok";
    pub const LAST_TEST_ERROR: &str = "email.last_test_error";
}

/// A complete email configuration loaded from the settings table.
/// The SMTP password is decrypted into plaintext for the sender's
/// use — it only lives in memory, never leaves the process.
#[derive(Debug, Clone, Default)]
pub struct DbEmailConfig {
    pub mode: String,
    pub from_address: String,
    pub from_name: String,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_username: String,
    pub smtp_password: String,
}

/// User-facing form: same shape as [`DbEmailConfig`] but without the
/// "last test" status fields. Used by the admin UI.
#[derive(Debug, Clone)]
pub struct UpdateEmailConfig {
    pub mode: String,
    pub from_address: String,
    pub from_name: String,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_username: String,
    /// None = leave existing password unchanged. Some(empty) = clear it.
    /// Some(nonempty) = encrypt and replace.
    pub smtp_password: Option<String>,
}

/// Keys for Discord integration settings.
pub mod discord_keys {
    pub const ENABLED: &str = "discord.enabled";
    pub const BOT_TOKEN: &str = "discord.bot_token";
    pub const GUILD_ID: &str = "discord.guild_id";
    pub const MEMBER_ROLE_ID: &str = "discord.member_role_id";
    pub const EXPIRED_ROLE_ID: &str = "discord.expired_role_id";
    pub const EVENTS_CHANNEL_ID: &str = "discord.events_channel_id";
    pub const ANNOUNCEMENTS_CHANNEL_ID: &str = "discord.announcements_channel_id";
    pub const ADMIN_ALERTS_CHANNEL_ID: &str = "discord.admin_alerts_channel_id";
    pub const INVITE_URL: &str = "discord.invite_url";
    pub const LAST_TEST_AT: &str = "discord.last_test_at";
    pub const LAST_TEST_OK: &str = "discord.last_test_ok";
    pub const LAST_TEST_ERROR: &str = "discord.last_test_error";
}

#[derive(Debug, Clone, Default)]
pub struct DbDiscordConfig {
    pub enabled: bool,
    pub bot_token: String,
    pub guild_id: String,
    pub member_role_id: String,
    pub expired_role_id: String,
    pub events_channel_id: String,
    pub announcements_channel_id: String,
    pub admin_alerts_channel_id: String,
    pub invite_url: String,
}

#[derive(Debug, Clone)]
pub struct UpdateDiscordConfig {
    pub enabled: bool,
    pub guild_id: String,
    pub member_role_id: String,
    pub expired_role_id: String,
    pub events_channel_id: String,
    pub announcements_channel_id: String,
    pub admin_alerts_channel_id: String,
    pub invite_url: String,
    /// None = leave existing token unchanged. Some(empty) = clear it.
    /// Some(nonempty) = encrypt and replace.
    pub bot_token: Option<String>,
}

/// Keys for membership/signup behavior settings.
pub mod membership_keys {
    pub const SIGNUP_MODE: &str = "membership.signup_mode";
    pub const SIGNUP_AUTO_RENEW: &str = "membership.signup_auto_renew";
}

/// The public-signup funnel mode (see the pay-at-signup spec).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignupMode {
    /// Signup creates a Pending member; an admin activates. Default.
    Approval,
    /// Signup returns a Stripe Checkout URL; a completed membership
    /// payment activates the member.
    Payment,
}

/// Keys for Stripe integration settings. DB-backed so an admin can add
/// or rotate Stripe credentials from the portal without a restart (the
/// old `integrations.stripe.*` toggle was dead — read nowhere).
pub mod stripe_keys {
    pub const ENABLED: &str = "stripe.enabled";
    pub const PUBLISHABLE_KEY: &str = "stripe.publishable_key";
    pub const SECRET_KEY: &str = "stripe.secret_key";
    pub const WEBHOOK_SECRET: &str = "stripe.webhook_secret";
    pub const SUCCESS_URL: &str = "stripe.success_url";
    pub const CANCEL_URL: &str = "stripe.cancel_url";
    pub const LAST_TEST_AT: &str = "stripe.last_test_at";
    pub const LAST_TEST_OK: &str = "stripe.last_test_ok";
    pub const LAST_TEST_ERROR: &str = "stripe.last_test_error";
}

/// Full Stripe configuration loaded from the settings table. The secret
/// key and webhook signing secret are decrypted into plaintext for
/// in-process use — they only live in memory, never leave the process.
#[derive(Debug, Clone, Default)]
pub struct DbStripeConfig {
    pub enabled: bool,
    pub publishable_key: String,
    pub secret_key: String,
    pub webhook_secret: String,
    pub success_url: String,
    pub cancel_url: String,
}

#[derive(Debug, Clone)]
pub struct UpdateStripeConfig {
    pub enabled: bool,
    pub publishable_key: String,
    pub success_url: String,
    pub cancel_url: String,
    /// None = leave existing secret key unchanged. Some(empty) = clear
    /// it. Some(nonempty) = encrypt and replace. (Mirror the SMTP
    /// password / Discord bot token convention.)
    pub secret_key: Option<String>,
    /// Same convention as `secret_key`, for the webhook signing secret.
    pub webhook_secret: Option<String>,
}

/// Keys for UniFi integration settings. DB-backed (mirrors Discord) so an
/// admin can add / rotate controller credentials from the portal without a
/// restart. The old `integrations.unifi.enabled` toggle was dead — read
/// nowhere; UniFi ran entirely from `.env`.
pub mod unifi_keys {
    pub const ENABLED: &str = "unifi.enabled";
    pub const CONTROLLER_URL: &str = "unifi.controller_url";
    pub const USERNAME: &str = "unifi.username";
    pub const PASSWORD: &str = "unifi.password";
    pub const SITE_ID: &str = "unifi.site_id";
    pub const LAST_TEST_AT: &str = "unifi.last_test_at";
    pub const LAST_TEST_OK: &str = "unifi.last_test_ok";
    pub const LAST_TEST_ERROR: &str = "unifi.last_test_error";
}

/// Full UniFi configuration loaded from the settings table. The password
/// is decrypted into plaintext for in-process use — it only lives in
/// memory, never leaves the process.
#[derive(Debug, Clone, Default)]
pub struct DbUnifiConfig {
    pub enabled: bool,
    pub controller_url: String,
    pub username: String,
    pub password: String,
    pub site_id: String,
}

#[derive(Debug, Clone)]
pub struct UpdateUnifiConfig {
    pub enabled: bool,
    pub controller_url: String,
    pub username: String,
    pub site_id: String,
    /// None = leave existing password unchanged. Some(empty) = clear it.
    /// Some(nonempty) = encrypt and replace. (Mirrors the SMTP password /
    /// Discord bot token / Stripe secret convention.)
    pub password: Option<String>,
}

/// Keys for bot-challenge (Turnstile) settings. DB-backed (mirrors
/// Stripe/Discord) so an admin can enable the captcha and set/rotate the
/// secret from the portal without a restart — the verifier reads these
/// live on every request.
pub mod bot_challenge_keys {
    pub const PROVIDER: &str = "bot_challenge.provider";
    pub const SECRET_KEY: &str = "bot_challenge.secret_key";
    pub const SITE_KEY: &str = "bot_challenge.site_key";
    pub const TIMEOUT_MS: &str = "bot_challenge.timeout_ms";
}

/// Bot-challenge configuration loaded from settings. The secret key is
/// decrypted into plaintext for the siteverify call — it only lives in
/// memory, never leaves the process, and is never logged.
#[derive(Debug, Clone, Default)]
pub struct DbBotChallengeConfig {
    /// `"disabled"` (no captcha) or `"turnstile"`.
    pub provider: String,
    /// Decrypted secret key.
    pub secret_key: String,
    /// Public site key (admin reference).
    pub site_key: String,
    pub timeout_ms: u64,
}

#[derive(FromRow)]
struct SettingRow {
    key: String,
    value: String,
    value_type: String,
    category: String,
    description: Option<String>,
    is_sensitive: bool,
    updated_by: Option<String>,
    updated_at: NaiveDateTime,
}

pub struct SettingsService {
    pool: SqlitePool,
    crypto: Arc<SecretCrypto>,
}

impl SettingsService {
    pub fn new(pool: SqlitePool, crypto: Arc<SecretCrypto>) -> Self {
        Self { pool, crypto }
    }

    pub async fn get_setting(&self, key: &str) -> Result<AppSetting> {
        let row = sqlx::query_as::<_, SettingRow>(
            r#"
            SELECT 
                key, value, value_type, category, description, 
                is_sensitive, updated_by, updated_at
            FROM app_settings
            WHERE key = ?
            "#,
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Setting not found: {}", key)))?;

        Ok(self.row_to_setting(row))
    }

    fn row_to_setting(&self, row: SettingRow) -> AppSetting {
        AppSetting {
            key: row.key,
            value: row.value,
            value_type: self.parse_setting_type(&row.value_type),
            category: row.category,
            description: row.description,
            is_sensitive: row.is_sensitive,
            updated_by: row.updated_by.and_then(|s| Uuid::parse_str(&s).ok()),
            updated_at: DateTime::from_naive_utc_and_offset(row.updated_at, Utc),
        }
    }

    fn parse_setting_type(&self, type_str: &str) -> SettingType {
        match type_str {
            "string" => SettingType::String,
            "number" => SettingType::Number,
            "boolean" => SettingType::Boolean,
            "json" => SettingType::Json,
            _ => SettingType::String,
        }
    }

    pub async fn get_all_settings(&self) -> Result<Vec<SettingsCategory>> {
        let rows = sqlx::query_as::<_, SettingRow>(
            r#"
            SELECT 
                key, value, value_type, category, description,
                is_sensitive, updated_by, updated_at
            FROM app_settings
            ORDER BY category, key
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        let settings: Vec<AppSetting> = rows.into_iter().map(|r| self.row_to_setting(r)).collect();

        // Group by category
        let mut categories: Vec<SettingsCategory> = Vec::new();
        let mut current_category: Option<SettingsCategory> = None;

        for setting in settings {
            match &mut current_category {
                Some(cat) if cat.name == setting.category => {
                    cat.settings.push(setting);
                }
                _ => {
                    if let Some(cat) = current_category.take() {
                        categories.push(cat);
                    }
                    current_category = Some(SettingsCategory {
                        name: setting.category.clone(),
                        settings: vec![setting],
                    });
                }
            }
        }

        if let Some(cat) = current_category {
            categories.push(cat);
        }

        Ok(categories)
    }

    pub async fn update_setting(
        &self,
        key: &str,
        request: UpdateSettingRequest,
        updated_by: Uuid,
    ) -> Result<AppSetting> {
        // Validate zone-typed settings before touching the DB, so a bad
        // value is rejected and the previous value is retained.
        if key == org_keys::TIMEZONE && request.value.parse::<chrono_tz::Tz>().is_err() {
            return Err(AppError::BadRequest(format!(
                "'{}' is not a recognized IANA timezone name",
                request.value
            )));
        }

        // Get the current setting first
        let current = self.get_setting(key).await?;

        // Sensitive settings are encrypted at rest (e.g.
        // `bot_challenge.secret_key` saved from the generic settings page)
        // and never appear in cleartext in the audit trail.
        let stored_value = if current.is_sensitive {
            self.crypto.encrypt(&request.value)?
        } else {
            request.value.clone()
        };
        let old_value = if current.is_sensitive {
            "[REDACTED]".to_string()
        } else {
            current.value.clone()
        };
        let new_value_for_audit = if current.is_sensitive {
            "[REDACTED]".to_string()
        } else {
            request.value.clone()
        };

        // Update the setting
        let now = Utc::now().naive_utc();
        sqlx::query(
            r#"
            UPDATE app_settings
            SET value = ?, updated_by = ?, updated_at = ?
            WHERE key = ?
            "#,
        )
        .bind(&stored_value)
        .bind(updated_by.to_string())
        .bind(now)
        .bind(key)
        .execute(&self.pool)
        .await?;

        // Create audit log entry
        let audit_id = Uuid::new_v4().to_string();
        sqlx::query(
            r#"
            INSERT INTO settings_audit (id, setting_key, old_value, new_value, changed_by, reason)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(audit_id)
        .bind(key)
        .bind(old_value)
        .bind(new_value_for_audit)
        .bind(updated_by.to_string())
        .bind(&request.reason)
        .execute(&self.pool)
        .await?;

        // Keep the process-cached `submissions.enabled` flag (read by the shared
        // portal layout to gate the admin nav link) in sync on write.
        if key == "submissions.enabled" {
            set_submissions_enabled_cached(request.value.parse().unwrap_or(false));
        }

        // Return updated setting
        self.get_setting(key).await
    }

    pub async fn get_value(&self, key: &str) -> Result<String> {
        let setting = self.get_setting(key).await?;
        Ok(setting.value)
    }

    /// The organization timezone as a parsed `Tz`, falling back to UTC
    /// when unset or (defensively) unparseable. UTC reproduces the
    /// pre-timezone behavior.
    pub async fn org_timezone(&self) -> chrono_tz::Tz {
        self.get_value(org_keys::TIMEZONE)
            .await
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(chrono_tz::Tz::UTC)
    }

    /// The signup funnel mode. Missing row or unrecognized value falls
    /// back to `Approval` (the pre-existing funnel), per the
    /// pay-at-signup spec — a broken setting must not silently open the
    /// payment path.
    pub async fn signup_mode(&self) -> SignupMode {
        match self.get_value(membership_keys::SIGNUP_MODE).await {
            Ok(v) if v == "payment" => SignupMode::Payment,
            _ => SignupMode::Approval,
        }
    }

    /// Whether paying signups are enrolled in auto-renew (card saved
    /// off-session, next renewal scheduled). Defaults to true on a
    /// missing/unparseable row — enrollment is the point of payment
    /// mode; orgs opt out explicitly.
    pub async fn signup_auto_renew(&self) -> bool {
        self.get_bool(membership_keys::SIGNUP_AUTO_RENEW)
            .await
            .unwrap_or(true)
    }

    pub async fn get_bool(&self, key: &str) -> Result<bool> {
        let value = self.get_value(key).await?;
        value
            .parse()
            .map_err(|_| AppError::Internal(format!("Invalid boolean value for {}", key)))
    }

    pub async fn get_number(&self, key: &str) -> Result<i64> {
        let value = self.get_value(key).await?;
        value
            .parse()
            .map_err(|_| AppError::Internal(format!("Invalid number value for {}", key)))
    }

    /// Load the full email configuration from the settings table,
    /// decrypting the SMTP password into plaintext.
    pub async fn get_email_config(&self) -> Result<DbEmailConfig> {
        let mode = self
            .get_value(email_keys::MODE)
            .await
            .unwrap_or_else(|_| "log".to_string());
        let from_address = self
            .get_value(email_keys::FROM_ADDRESS)
            .await
            .unwrap_or_default();
        let from_name = self
            .get_value(email_keys::FROM_NAME)
            .await
            .unwrap_or_else(|_| "Coterie".to_string());
        let smtp_host = self
            .get_value(email_keys::SMTP_HOST)
            .await
            .unwrap_or_default();
        let smtp_port = self.get_number(email_keys::SMTP_PORT).await.unwrap_or(587) as u16;
        let smtp_username = self
            .get_value(email_keys::SMTP_USERNAME)
            .await
            .unwrap_or_default();
        let encrypted_password = self
            .get_value(email_keys::SMTP_PASSWORD)
            .await
            .unwrap_or_default();
        let smtp_password = self.crypto.decrypt(&encrypted_password)?;

        Ok(DbEmailConfig {
            mode,
            from_address,
            from_name,
            smtp_host,
            smtp_port,
            smtp_username,
            smtp_password,
        })
    }

    /// Whether a real email provider is configured (SMTP mode with a
    /// host and from-address). Used to gate outgoing receipt emails: the
    /// `log` mode is a dev/no-op sink, so it does not count as
    /// configured. Never errors — an unreadable config reads as "not
    /// configured" so a missing provider silently skips the send rather
    /// than failing the payment.
    pub async fn is_email_configured(&self) -> bool {
        match self.get_email_config().await {
            Ok(cfg) => {
                cfg.mode == "smtp" && !cfg.smtp_host.is_empty() && !cfg.from_address.is_empty()
            }
            Err(_) => false,
        }
    }

    /// Returns `true` if the stored SMTP password exists but can't be
    /// decrypted — almost always a sign that `session_secret` was
    /// rotated. The admin UI uses this to show a clear warning banner.
    pub async fn smtp_password_undecryptable(&self) -> bool {
        let encrypted = self
            .get_value(email_keys::SMTP_PASSWORD)
            .await
            .unwrap_or_default();
        if encrypted.is_empty() {
            return false;
        }
        self.crypto.decrypt(&encrypted).is_err()
    }

    /// Persist an updated email configuration. Encrypts the SMTP
    /// password before storage; leaves it unchanged when `smtp_password`
    /// is `None` (e.g. the form was submitted without re-typing it).
    pub async fn update_email_config(
        &self,
        config: UpdateEmailConfig,
        updated_by: Uuid,
    ) -> Result<()> {
        self.set_value_raw(email_keys::MODE, &config.mode, updated_by)
            .await?;
        self.set_value_raw(email_keys::FROM_ADDRESS, &config.from_address, updated_by)
            .await?;
        self.set_value_raw(email_keys::FROM_NAME, &config.from_name, updated_by)
            .await?;
        self.set_value_raw(email_keys::SMTP_HOST, &config.smtp_host, updated_by)
            .await?;
        self.set_value_raw(
            email_keys::SMTP_PORT,
            &config.smtp_port.to_string(),
            updated_by,
        )
        .await?;
        self.set_value_raw(email_keys::SMTP_USERNAME, &config.smtp_username, updated_by)
            .await?;

        if let Some(new_password) = config.smtp_password {
            let encrypted = self.crypto.encrypt(&new_password)?;
            self.set_value_raw(email_keys::SMTP_PASSWORD, &encrypted, updated_by)
                .await?;
        }

        Ok(())
    }

    /// Load the full Discord integration configuration. Bot token is
    /// decrypted into plaintext for the integration's use.
    pub async fn get_discord_config(&self) -> Result<DbDiscordConfig> {
        let enabled = self.get_bool(discord_keys::ENABLED).await.unwrap_or(false);
        let guild_id = self
            .get_value(discord_keys::GUILD_ID)
            .await
            .unwrap_or_default();
        let member_role_id = self
            .get_value(discord_keys::MEMBER_ROLE_ID)
            .await
            .unwrap_or_default();
        let expired_role_id = self
            .get_value(discord_keys::EXPIRED_ROLE_ID)
            .await
            .unwrap_or_default();
        let events_channel_id = self
            .get_value(discord_keys::EVENTS_CHANNEL_ID)
            .await
            .unwrap_or_default();
        let announcements_channel_id = self
            .get_value(discord_keys::ANNOUNCEMENTS_CHANNEL_ID)
            .await
            .unwrap_or_default();
        let admin_alerts_channel_id = self
            .get_value(discord_keys::ADMIN_ALERTS_CHANNEL_ID)
            .await
            .unwrap_or_default();
        let invite_url = self
            .get_value(discord_keys::INVITE_URL)
            .await
            .unwrap_or_default();
        let encrypted = self
            .get_value(discord_keys::BOT_TOKEN)
            .await
            .unwrap_or_default();
        let bot_token = self.crypto.decrypt(&encrypted)?;

        Ok(DbDiscordConfig {
            enabled,
            bot_token,
            guild_id,
            member_role_id,
            expired_role_id,
            events_channel_id,
            announcements_channel_id,
            admin_alerts_channel_id,
            invite_url,
        })
    }

    /// True if the encrypted bot token exists but won't decrypt — same
    /// shape as `smtp_password_undecryptable`. Triggers the admin UI's
    /// rotation banner.
    pub async fn discord_token_undecryptable(&self) -> bool {
        let encrypted = self
            .get_value(discord_keys::BOT_TOKEN)
            .await
            .unwrap_or_default();
        if encrypted.is_empty() {
            return false;
        }
        self.crypto.decrypt(&encrypted).is_err()
    }

    pub async fn update_discord_config(
        &self,
        config: UpdateDiscordConfig,
        updated_by: Uuid,
    ) -> Result<()> {
        self.set_value_raw(
            discord_keys::ENABLED,
            if config.enabled { "true" } else { "false" },
            updated_by,
        )
        .await?;
        self.set_value_raw(discord_keys::GUILD_ID, &config.guild_id, updated_by)
            .await?;
        self.set_value_raw(
            discord_keys::MEMBER_ROLE_ID,
            &config.member_role_id,
            updated_by,
        )
        .await?;
        self.set_value_raw(
            discord_keys::EXPIRED_ROLE_ID,
            &config.expired_role_id,
            updated_by,
        )
        .await?;
        self.set_value_raw(
            discord_keys::EVENTS_CHANNEL_ID,
            &config.events_channel_id,
            updated_by,
        )
        .await?;
        self.set_value_raw(
            discord_keys::ANNOUNCEMENTS_CHANNEL_ID,
            &config.announcements_channel_id,
            updated_by,
        )
        .await?;
        self.set_value_raw(
            discord_keys::ADMIN_ALERTS_CHANNEL_ID,
            &config.admin_alerts_channel_id,
            updated_by,
        )
        .await?;
        self.set_value_raw(discord_keys::INVITE_URL, &config.invite_url, updated_by)
            .await?;

        if let Some(new_token) = config.bot_token {
            let encrypted = self.crypto.encrypt(&new_token)?;
            self.set_value_raw(discord_keys::BOT_TOKEN, &encrypted, updated_by)
                .await?;
        }

        Ok(())
    }

    pub async fn record_discord_test(&self, ok: bool, error: &str, updated_by: Uuid) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.set_value_raw(discord_keys::LAST_TEST_AT, &now, updated_by)
            .await?;
        self.set_value_raw(
            discord_keys::LAST_TEST_OK,
            if ok { "true" } else { "false" },
            updated_by,
        )
        .await?;
        self.set_value_raw(discord_keys::LAST_TEST_ERROR, error, updated_by)
            .await?;
        Ok(())
    }

    /// Load the full Stripe configuration. The secret key and webhook
    /// signing secret are decrypted into plaintext for use. A decrypt
    /// failure (e.g. session_secret was rotated) surfaces as `Err` —
    /// callers treat that as "unconfigured" and the settings page shows
    /// the failure, the same fail-safe as Discord/email.
    pub async fn get_stripe_config(&self) -> Result<DbStripeConfig> {
        let enabled = self.get_bool(stripe_keys::ENABLED).await.unwrap_or(false);
        let publishable_key = self
            .get_value(stripe_keys::PUBLISHABLE_KEY)
            .await
            .unwrap_or_default();
        let success_url = self
            .get_value(stripe_keys::SUCCESS_URL)
            .await
            .unwrap_or_default();
        let cancel_url = self
            .get_value(stripe_keys::CANCEL_URL)
            .await
            .unwrap_or_default();
        let encrypted_secret = self
            .get_value(stripe_keys::SECRET_KEY)
            .await
            .unwrap_or_default();
        let secret_key = self.crypto.decrypt(&encrypted_secret)?;
        let encrypted_webhook = self
            .get_value(stripe_keys::WEBHOOK_SECRET)
            .await
            .unwrap_or_default();
        let webhook_secret = self.crypto.decrypt(&encrypted_webhook)?;

        Ok(DbStripeConfig {
            enabled,
            publishable_key,
            secret_key,
            webhook_secret,
            success_url,
            cancel_url,
        })
    }

    /// True if an encrypted Stripe secret exists but won't decrypt —
    /// same shape as `smtp_password_undecryptable` /
    /// `discord_token_undecryptable`. Drives the admin UI's rotation
    /// warning banner.
    pub async fn stripe_secret_undecryptable(&self) -> bool {
        for key in [stripe_keys::SECRET_KEY, stripe_keys::WEBHOOK_SECRET] {
            let encrypted = self.get_value(key).await.unwrap_or_default();
            if !encrypted.is_empty() && self.crypto.decrypt(&encrypted).is_err() {
                return true;
            }
        }
        false
    }

    /// Persist updated Stripe configuration. The secret key and webhook
    /// signing secret are encrypted before storage; each is left
    /// unchanged when its field is `None` (form submitted blank).
    pub async fn update_stripe_config(
        &self,
        config: UpdateStripeConfig,
        updated_by: Uuid,
    ) -> Result<()> {
        self.set_value_raw(
            stripe_keys::ENABLED,
            if config.enabled { "true" } else { "false" },
            updated_by,
        )
        .await?;
        self.set_value_raw(
            stripe_keys::PUBLISHABLE_KEY,
            &config.publishable_key,
            updated_by,
        )
        .await?;
        self.set_value_raw(stripe_keys::SUCCESS_URL, &config.success_url, updated_by)
            .await?;
        self.set_value_raw(stripe_keys::CANCEL_URL, &config.cancel_url, updated_by)
            .await?;

        if let Some(new_secret) = config.secret_key {
            let encrypted = self.crypto.encrypt(&new_secret)?;
            self.set_value_raw(stripe_keys::SECRET_KEY, &encrypted, updated_by)
                .await?;
        }
        if let Some(new_webhook) = config.webhook_secret {
            let encrypted = self.crypto.encrypt(&new_webhook)?;
            self.set_value_raw(stripe_keys::WEBHOOK_SECRET, &encrypted, updated_by)
                .await?;
        }

        Ok(())
    }

    /// True if the DB already carries meaningful Stripe config — i.e.
    /// anyone has enabled Stripe or stored any of the three keys. Used
    /// by the one-time `.env` seed to decide whether the database is
    /// still pristine. Reads the raw (still-encrypted) secret values, so
    /// it doesn't care whether they decrypt.
    pub async fn has_stripe_config(&self) -> bool {
        if self.get_bool(stripe_keys::ENABLED).await.unwrap_or(false) {
            return true;
        }
        for key in [
            stripe_keys::PUBLISHABLE_KEY,
            stripe_keys::SECRET_KEY,
            stripe_keys::WEBHOOK_SECRET,
        ] {
            if !self
                .get_value(key)
                .await
                .unwrap_or_default()
                .trim()
                .is_empty()
            {
                return true;
            }
        }
        false
    }

    /// One-time `.env` → DB seed. When the database holds no Stripe
    /// config but the provisioning environment provides a secret key,
    /// copy the env values in (encrypting the secrets) so wizard/IaC
    /// installs come up configured. Returns `true` if it seeded.
    /// Thereafter `has_stripe_config` is true and this is a no-op — the
    /// database is authoritative and env Stripe values are ignored.
    pub async fn seed_stripe_from_env(
        &self,
        env: &crate::config::StripeConfig,
        updated_by: Uuid,
    ) -> Result<bool> {
        let env_secret = crate::config::nonblank(env.secret_key.clone());
        // Nothing to seed from, or the DB is already configured.
        if env_secret.is_none() || self.has_stripe_config().await {
            return Ok(false);
        }

        self.update_stripe_config(
            UpdateStripeConfig {
                enabled: env.enabled,
                publishable_key: env.publishable_key.clone().unwrap_or_default(),
                success_url: String::new(),
                cancel_url: String::new(),
                secret_key: env_secret,
                webhook_secret: crate::config::nonblank(env.webhook_secret.clone()),
            },
            updated_by,
        )
        .await?;
        tracing::info!(
            "Seeded Stripe configuration from environment into the database (one-time); \
             the database is now authoritative and .env Stripe values are ignored"
        );
        Ok(true)
    }

    /// Record the result of a Stripe connection test so the admin UI can
    /// show health at a glance.
    pub async fn record_stripe_test(&self, ok: bool, error: &str, updated_by: Uuid) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.set_value_raw(stripe_keys::LAST_TEST_AT, &now, updated_by)
            .await?;
        self.set_value_raw(
            stripe_keys::LAST_TEST_OK,
            if ok { "true" } else { "false" },
            updated_by,
        )
        .await?;
        self.set_value_raw(stripe_keys::LAST_TEST_ERROR, error, updated_by)
            .await?;
        Ok(())
    }

    /// Load the full UniFi configuration. The password is decrypted into
    /// plaintext for the integration's use. A decrypt failure (e.g.
    /// session_secret was rotated) surfaces as `Err` — callers treat that
    /// as "unconfigured" and the settings page shows the failure, the same
    /// fail-safe as Stripe/Discord/email.
    pub async fn get_unifi_config(&self) -> Result<DbUnifiConfig> {
        let enabled = self.get_bool(unifi_keys::ENABLED).await.unwrap_or(false);
        let controller_url = self
            .get_value(unifi_keys::CONTROLLER_URL)
            .await
            .unwrap_or_default();
        let username = self
            .get_value(unifi_keys::USERNAME)
            .await
            .unwrap_or_default();
        let site_id = self
            .get_value(unifi_keys::SITE_ID)
            .await
            .unwrap_or_default();
        let encrypted_password = self
            .get_value(unifi_keys::PASSWORD)
            .await
            .unwrap_or_default();
        let password = self.crypto.decrypt(&encrypted_password)?;

        Ok(DbUnifiConfig {
            enabled,
            controller_url,
            username,
            password,
            site_id,
        })
    }

    /// True if an encrypted UniFi password exists but won't decrypt — same
    /// shape as `stripe_secret_undecryptable`. Drives the admin UI's
    /// rotation warning banner.
    pub async fn unifi_password_undecryptable(&self) -> bool {
        let encrypted = self
            .get_value(unifi_keys::PASSWORD)
            .await
            .unwrap_or_default();
        if encrypted.is_empty() {
            return false;
        }
        self.crypto.decrypt(&encrypted).is_err()
    }

    /// Persist updated UniFi configuration. The password is encrypted
    /// before storage; it's left unchanged when `password` is `None` (form
    /// submitted blank).
    pub async fn update_unifi_config(
        &self,
        config: UpdateUnifiConfig,
        updated_by: Uuid,
    ) -> Result<()> {
        self.set_value_raw(
            unifi_keys::ENABLED,
            if config.enabled { "true" } else { "false" },
            updated_by,
        )
        .await?;
        self.set_value_raw(
            unifi_keys::CONTROLLER_URL,
            &config.controller_url,
            updated_by,
        )
        .await?;
        self.set_value_raw(unifi_keys::USERNAME, &config.username, updated_by)
            .await?;
        self.set_value_raw(unifi_keys::SITE_ID, &config.site_id, updated_by)
            .await?;

        if let Some(new_password) = config.password {
            let encrypted = self.crypto.encrypt(&new_password)?;
            self.set_value_raw(unifi_keys::PASSWORD, &encrypted, updated_by)
                .await?;
        }

        Ok(())
    }

    /// True if the DB already carries meaningful UniFi config — i.e. anyone
    /// has enabled UniFi or stored a controller URL or password. Used by
    /// the one-time `.env` seed to decide whether the database is still
    /// pristine. Reads the raw (still-encrypted) password, so it doesn't
    /// care whether it decrypts.
    pub async fn has_unifi_config(&self) -> bool {
        if self.get_bool(unifi_keys::ENABLED).await.unwrap_or(false) {
            return true;
        }
        for key in [unifi_keys::CONTROLLER_URL, unifi_keys::PASSWORD] {
            if !self
                .get_value(key)
                .await
                .unwrap_or_default()
                .trim()
                .is_empty()
            {
                return true;
            }
        }
        false
    }

    /// One-time `.env` → DB seed. When the database holds no UniFi config
    /// but the provisioning environment provides a controller URL, copy the
    /// env values in (encrypting the password) so wizard/IaC installs come
    /// up configured. Returns `true` if it seeded. Thereafter
    /// `has_unifi_config` is true and this is a no-op — the database is
    /// authoritative and env UniFi values are ignored.
    pub async fn seed_unifi_from_env(
        &self,
        env: &crate::config::UnifiConfig,
        updated_by: Uuid,
    ) -> Result<bool> {
        // Nothing to seed from, or the DB is already configured.
        if env.controller_url.trim().is_empty() || self.has_unifi_config().await {
            return Ok(false);
        }

        let password = if env.password.trim().is_empty() {
            None
        } else {
            Some(env.password.clone())
        };
        self.update_unifi_config(
            UpdateUnifiConfig {
                enabled: env.enabled,
                controller_url: env.controller_url.clone(),
                username: env.username.clone(),
                site_id: env.site_id.clone(),
                password,
            },
            updated_by,
        )
        .await?;
        tracing::info!(
            "Seeded UniFi configuration from environment into the database (one-time); \
             the database is now authoritative and .env UniFi values are ignored"
        );
        Ok(true)
    }

    /// Record the result of a UniFi connection test so the admin UI can
    /// show health at a glance.
    pub async fn record_unifi_test(&self, ok: bool, error: &str, updated_by: Uuid) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.set_value_raw(unifi_keys::LAST_TEST_AT, &now, updated_by)
            .await?;
        self.set_value_raw(
            unifi_keys::LAST_TEST_OK,
            if ok { "true" } else { "false" },
            updated_by,
        )
        .await?;
        self.set_value_raw(unifi_keys::LAST_TEST_ERROR, error, updated_by)
            .await?;
        Ok(())
    }

    /// Record the result of a test-email attempt so the admin UI can
    /// show health at a glance.
    pub async fn record_email_test(&self, ok: bool, error: &str, updated_by: Uuid) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.set_value_raw(email_keys::LAST_TEST_AT, &now, updated_by)
            .await?;
        self.set_value_raw(
            email_keys::LAST_TEST_OK,
            if ok { "true" } else { "false" },
            updated_by,
        )
        .await?;
        self.set_value_raw(email_keys::LAST_TEST_ERROR, error, updated_by)
            .await?;
        Ok(())
    }

    /// Load the bot-challenge configuration. The secret key is decrypted
    /// into plaintext for the siteverify call. A decrypt failure (e.g.
    /// `session_secret` was rotated) surfaces as `Err`; the verifier
    /// treats that as fail-closed when a provider is active, mirroring
    /// Stripe/Discord/UniFi.
    pub async fn get_bot_challenge_config(&self) -> Result<DbBotChallengeConfig> {
        let provider = self
            .get_value(bot_challenge_keys::PROVIDER)
            .await
            .unwrap_or_else(|_| "disabled".to_string());
        let site_key = self
            .get_value(bot_challenge_keys::SITE_KEY)
            .await
            .unwrap_or_default();
        let timeout_ms = self
            .get_number(bot_challenge_keys::TIMEOUT_MS)
            .await
            .unwrap_or(5000)
            .max(0) as u64;
        let encrypted_secret = self
            .get_value(bot_challenge_keys::SECRET_KEY)
            .await
            .unwrap_or_default();
        let secret_key = self.crypto.decrypt(&encrypted_secret)?;

        Ok(DbBotChallengeConfig {
            provider,
            secret_key,
            site_key,
            timeout_ms,
        })
    }

    /// Write a setting value directly without going through the audit
    /// log (used for bulk updates like `update_email_config` and for
    /// system-recorded state like test-result timestamps).
    ///
    /// `updated_by` is the acting member. The nil UUID is the "system"
    /// actor (e.g. the one-time `.env` Stripe seed at startup, which
    /// runs before any admin exists) and is stored as NULL — binding a
    /// non-existent member id would violate the `updated_by` FK.
    async fn set_value_raw(&self, key: &str, value: &str, updated_by: Uuid) -> Result<()> {
        let now = Utc::now().naive_utc();
        let updated_by = if updated_by.is_nil() {
            None
        } else {
            Some(updated_by.to_string())
        };
        sqlx::query(
            "UPDATE app_settings SET value = ?, updated_by = ?, updated_at = ? WHERE key = ?",
        )
        .bind(value)
        .bind(updated_by)
        .bind(now)
        .bind(key)
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(())
    }
}
