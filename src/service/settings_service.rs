use std::sync::Arc;

use chrono::{Utc, NaiveDateTime, DateTime};
use uuid::Uuid;
use sqlx::{SqlitePool, FromRow};

use crate::{
    auth::SecretCrypto,
    domain::{AppSetting, UpdateSettingRequest, SettingsCategory, PaymentConfig, MembershipConfig, SettingType},
    error::{AppError, Result},
};

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
            "#
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

    pub async fn get_settings_by_category(&self, category: &str) -> Result<Vec<AppSetting>> {
        let rows = sqlx::query_as::<_, SettingRow>(
            r#"
            SELECT 
                key, value, value_type, category, description,
                is_sensitive, updated_by, updated_at
            FROM app_settings
            WHERE category = ?
            ORDER BY key
            "#
        )
        .bind(category)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| self.row_to_setting(r)).collect())
    }

    pub async fn get_all_settings(&self) -> Result<Vec<SettingsCategory>> {
        let rows = sqlx::query_as::<_, SettingRow>(
            r#"
            SELECT 
                key, value, value_type, category, description,
                is_sensitive, updated_by, updated_at
            FROM app_settings
            ORDER BY category, key
            "#
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
        // Get the current setting first
        let current = self.get_setting(key).await?;

        // Don't return sensitive values in audit logs
        let old_value = if current.is_sensitive {
            "[REDACTED]".to_string()
        } else {
            current.value.clone()
        };

        // Update the setting
        let now = Utc::now().naive_utc();
        sqlx::query(
            r#"
            UPDATE app_settings
            SET value = ?, updated_by = ?, updated_at = ?
            WHERE key = ?
            "#
        )
        .bind(&request.value)
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
            "#
        )
        .bind(audit_id)
        .bind(key)
        .bind(old_value)
        .bind(&request.value)
        .bind(updated_by.to_string())
        .bind(&request.reason)
        .execute(&self.pool)
        .await?;

        // Return updated setting
        self.get_setting(key).await
    }

    pub async fn get_payment_config(&self) -> Result<PaymentConfig> {
        let settings = self.get_settings_by_category("payment").await?;
        Ok(PaymentConfig::from_settings(&settings))
    }

    pub async fn get_membership_config(&self) -> Result<MembershipConfig> {
        let settings = self.get_settings_by_category("membership").await?;
        Ok(MembershipConfig::from_settings(&settings))
    }

    pub async fn get_value(&self, key: &str) -> Result<String> {
        let setting = self.get_setting(key).await?;
        Ok(setting.value)
    }

    pub async fn get_bool(&self, key: &str) -> Result<bool> {
        let value = self.get_value(key).await?;
        value.parse().map_err(|_| AppError::Internal(format!("Invalid boolean value for {}", key)))
    }

    pub async fn get_number(&self, key: &str) -> Result<i64> {
        let value = self.get_value(key).await?;
        value.parse().map_err(|_| AppError::Internal(format!("Invalid number value for {}", key)))
    }

    /// Load the full email configuration from the settings table,
    /// decrypting the SMTP password into plaintext.
    pub async fn get_email_config(&self) -> Result<DbEmailConfig> {
        let mode = self.get_value(email_keys::MODE).await.unwrap_or_else(|_| "log".to_string());
        let from_address = self.get_value(email_keys::FROM_ADDRESS).await.unwrap_or_default();
        let from_name = self.get_value(email_keys::FROM_NAME).await.unwrap_or_else(|_| "Coterie".to_string());
        let smtp_host = self.get_value(email_keys::SMTP_HOST).await.unwrap_or_default();
        let smtp_port = self.get_number(email_keys::SMTP_PORT).await.unwrap_or(587) as u16;
        let smtp_username = self.get_value(email_keys::SMTP_USERNAME).await.unwrap_or_default();
        let encrypted_password = self.get_value(email_keys::SMTP_PASSWORD).await.unwrap_or_default();
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

    /// Persist an updated email configuration. Encrypts the SMTP
    /// password before storage; leaves it unchanged when `smtp_password`
    /// is `None` (e.g. the form was submitted without re-typing it).
    pub async fn update_email_config(
        &self,
        config: UpdateEmailConfig,
        updated_by: Uuid,
    ) -> Result<()> {
        self.set_value_raw(email_keys::MODE, &config.mode, updated_by).await?;
        self.set_value_raw(email_keys::FROM_ADDRESS, &config.from_address, updated_by).await?;
        self.set_value_raw(email_keys::FROM_NAME, &config.from_name, updated_by).await?;
        self.set_value_raw(email_keys::SMTP_HOST, &config.smtp_host, updated_by).await?;
        self.set_value_raw(email_keys::SMTP_PORT, &config.smtp_port.to_string(), updated_by).await?;
        self.set_value_raw(email_keys::SMTP_USERNAME, &config.smtp_username, updated_by).await?;

        if let Some(new_password) = config.smtp_password {
            let encrypted = self.crypto.encrypt(&new_password)?;
            self.set_value_raw(email_keys::SMTP_PASSWORD, &encrypted, updated_by).await?;
        }

        Ok(())
    }

    /// Record the result of a test-email attempt so the admin UI can
    /// show health at a glance.
    pub async fn record_email_test(&self, ok: bool, error: &str, updated_by: Uuid) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.set_value_raw(email_keys::LAST_TEST_AT, &now, updated_by).await?;
        self.set_value_raw(email_keys::LAST_TEST_OK, if ok { "true" } else { "false" }, updated_by).await?;
        self.set_value_raw(email_keys::LAST_TEST_ERROR, error, updated_by).await?;
        Ok(())
    }

    /// Write a setting value directly without going through the audit
    /// log (used for bulk updates like `update_email_config` and for
    /// system-recorded state like test-result timestamps).
    async fn set_value_raw(&self, key: &str, value: &str, updated_by: Uuid) -> Result<()> {
        let now = Utc::now().naive_utc();
        sqlx::query(
            "UPDATE app_settings SET value = ?, updated_by = ?, updated_at = ? WHERE key = ?"
        )
        .bind(value)
        .bind(updated_by.to_string())
        .bind(now)
        .bind(key)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }
}