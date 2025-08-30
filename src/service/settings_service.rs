use chrono::{Utc, NaiveDateTime, DateTime};
use uuid::Uuid;
use sqlx::{SqlitePool, FromRow};

use crate::{
    domain::{AppSetting, UpdateSettingRequest, SettingsCategory, PaymentConfig, MembershipConfig, SettingType},
    error::{AppError, Result},
};

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
}

impl SettingsService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
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
}