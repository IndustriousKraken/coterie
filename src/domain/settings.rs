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

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
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

// Helper struct for payment configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentConfig {
    pub regular_membership_fee: i64,
    pub student_membership_fee: i64,
    pub corporate_membership_fee: i64,
    pub lifetime_membership_fee: i64,
    pub grace_period_days: i32,
    pub reminder_days_before: i32,
}

impl PaymentConfig {
    pub fn from_settings(settings: &[AppSetting]) -> Self {
        let mut config = PaymentConfig {
            regular_membership_fee: 5000,
            student_membership_fee: 2500,
            corporate_membership_fee: 50000,
            lifetime_membership_fee: 100000,
            grace_period_days: 30,
            reminder_days_before: 7,
        };

        for setting in settings {
            match setting.key.as_str() {
                "payment.regular_membership_fee" => {
                    config.regular_membership_fee = setting.value.parse().unwrap_or(5000);
                }
                "payment.student_membership_fee" => {
                    config.student_membership_fee = setting.value.parse().unwrap_or(2500);
                }
                "payment.corporate_membership_fee" => {
                    config.corporate_membership_fee = setting.value.parse().unwrap_or(50000);
                }
                "payment.lifetime_membership_fee" => {
                    config.lifetime_membership_fee = setting.value.parse().unwrap_or(100000);
                }
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