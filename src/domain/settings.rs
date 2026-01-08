use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSetting {
    pub key: String,
    pub value: String,
    pub value_type: SettingType,
    pub category: String,
    pub description: Option<String>,
    pub is_sensitive: bool,
    pub updated_by: Option<Uuid>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq)]
#[sqlx(type_name = "TEXT")]
pub enum SettingType {
    #[serde(rename = "string")]
    String,
    #[serde(rename = "number")]
    Number,
    #[serde(rename = "boolean")]
    Boolean,
    #[serde(rename = "json")]
    Json,
}

impl SettingType {
    pub fn as_str(&self) -> &'static str {
        match self {
            SettingType::String => "string",
            SettingType::Number => "number", 
            SettingType::Boolean => "boolean",
            SettingType::Json => "json",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSettingRequest {
    pub value: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsCategory {
    pub name: String,
    pub settings: Vec<AppSetting>,
}

/// Payment configuration for general payment settings.
///
/// Note: Per-membership-type fees are now stored in the `membership_types` table
/// and managed via the admin UI at /portal/admin/types. Use `MembershipTypeConfig.fee_cents`
/// to get the fee for a specific membership type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentConfig {
    /// Days after dues expiry before member status changes
    pub grace_period_days: i32,
    /// Days before dues expiry to send reminder notifications
    pub reminder_days_before: i32,
}

impl PaymentConfig {
    pub fn from_settings(settings: &[AppSetting]) -> Self {
        let mut config = PaymentConfig {
            grace_period_days: 30,
            reminder_days_before: 7,
        };

        for setting in settings {
            match setting.key.as_str() {
                "payment.grace_period_days" => {
                    config.grace_period_days = setting.value.parse().unwrap_or(30);
                }
                "payment.reminder_days_before" => {
                    config.reminder_days_before = setting.value.parse().unwrap_or(7);
                }
                _ => {}
            }
        }

        config
    }
}

// Helper struct for membership configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MembershipConfig {
    pub auto_approve: bool,
    pub require_payment_for_activation: bool,
    pub default_duration_months: i32,
}

impl MembershipConfig {
    pub fn from_settings(settings: &[AppSetting]) -> Self {
        let mut config = MembershipConfig {
            auto_approve: false,
            require_payment_for_activation: true,
            default_duration_months: 12,
        };

        for setting in settings {
            match setting.key.as_str() {
                "membership.auto_approve" => {
                    config.auto_approve = setting.value.parse().unwrap_or(false);
                }
                "membership.require_payment_for_activation" => {
                    config.require_payment_for_activation = setting.value.parse().unwrap_or(true);
                }
                "membership.default_duration_months" => {
                    config.default_duration_months = setting.value.parse().unwrap_or(12);
                }
                _ => {}
            }
        }

        config
    }
}