use std::sync::Arc;

use askama::Template;
use axum::{
    extract::State,
    response::{IntoResponse, Response},
    Extension, Form,
};
use serde::Deserialize;

use crate::{
    api::middleware::auth::{CurrentUser, SessionInfo},
    auth::CsrfService,
    domain::{AppSetting, UpdateSettingRequest},
    service::{audit_service::AuditService, settings_service::SettingsService},
    web::templates::{BaseContext, HtmlTemplate},
};

// =============================================================================
// Template Structs
// =============================================================================

/// Setting info for template display
#[derive(Clone)]
pub struct SettingInfo {
    pub key: String,
    pub display_name: String,
    pub value: String,
    pub value_type: String,
    pub description: Option<String>,
    pub is_sensitive: bool,
    /// True for `org.timezone` — the template renders a zone dropdown
    /// instead of a free-text field. `timezone_options` holds the
    /// choices (current value first, marked selected).
    pub is_timezone: bool,
    pub timezone_options: Vec<TzOption>,
    /// True for `membership.signup_mode` — renders an approval/payment
    /// dropdown instead of a free-text field. Reuses `TzOption` (value +
    /// selected); the values are `SignupMode`'s wire strings.
    pub is_signup_mode: bool,
    pub signup_mode_options: Vec<TzOption>,
    /// True for `bot_challenge.provider` — renders a disabled/turnstile
    /// dropdown (same mechanism as `is_signup_mode`).
    pub is_bot_challenge_provider: bool,
    pub bot_challenge_provider_options: Vec<TzOption>,
}

/// One option in the timezone dropdown.
#[derive(Clone)]
pub struct TzOption {
    pub value: String,
    pub selected: bool,
}

/// Common IANA zones offered in the settings dropdown. Not exhaustive —
/// a stored value outside this list is prepended so it stays selectable.
const COMMON_TIMEZONES: &[&str] = &[
    "UTC",
    "America/New_York",
    "America/Chicago",
    "America/Denver",
    "America/Phoenix",
    "America/Los_Angeles",
    "America/Anchorage",
    "Pacific/Honolulu",
    "America/Toronto",
    "America/Sao_Paulo",
    "Europe/London",
    "Europe/Paris",
    "Europe/Berlin",
    "Europe/Madrid",
    "Europe/Moscow",
    "Asia/Kolkata",
    "Asia/Dubai",
    "Asia/Shanghai",
    "Asia/Singapore",
    "Asia/Tokyo",
    "Australia/Sydney",
    "Pacific/Auckland",
];

/// Category of settings for template display
#[derive(Clone)]
pub struct SettingsCategoryInfo {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub settings: Vec<SettingInfo>,
}

#[derive(Template)]
#[template(path = "admin/settings.html")]
pub struct AdminSettingsTemplate {
    pub base: BaseContext,
    pub categories: Vec<SettingsCategoryInfo>,
    pub success_message: Option<String>,
    pub error_message: Option<String>,
}

// =============================================================================
// Form Structs
// =============================================================================

#[derive(Debug, Deserialize)]
pub struct UpdateSettingForm {
    pub csrf_token: String,
    pub setting_key: String,
    pub setting_value: String,
}

// =============================================================================
// Handlers
// =============================================================================

pub async fn admin_settings_page(
    State(settings_service): State<Arc<SettingsService>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> impl IntoResponse {
    admin_settings_page_inner(
        &settings_service,
        &csrf_service,
        &current_user,
        &session_info,
        None,
        None,
    )
    .await
}

async fn admin_settings_page_inner(
    settings_service: &SettingsService,
    csrf_service: &CsrfService,
    current_user: &CurrentUser,
    session_info: &SessionInfo,
    success_message: Option<String>,
    error_message: Option<String>,
) -> Response {
    let base = BaseContext::for_member(csrf_service, current_user, session_info).await;

    let categories = fetch_settings_by_category(settings_service).await;

    HtmlTemplate(AdminSettingsTemplate {
        base,
        categories,
        success_message,
        error_message,
    })
    .into_response()
}

pub async fn admin_update_setting(
    State(settings_service): State<Arc<SettingsService>>,
    State(csrf_service): State<Arc<CsrfService>>,
    State(audit_service): State<Arc<AuditService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Form(form): Form<UpdateSettingForm>,
) -> impl IntoResponse {
    // Validate CSRF
    let csrf_valid = csrf_service
        .validate_token(&session_info.session_id, &form.csrf_token)
        .await
        .unwrap_or(false);

    if !csrf_valid {
        return admin_settings_page_inner(
            &settings_service,
            &csrf_service,
            &current_user,
            &session_info,
            None,
            Some("Invalid CSRF token. Please try again.".to_string()),
        )
        .await;
    }

    // Capture the old value (before the update) so the audit-log diff
    // shows "was X, now Y". Sensitive settings get [REDACTED] on both
    // sides — we don't want SMTP passwords or similar in the log.
    let prior = settings_service.get_setting(&form.setting_key).await.ok();
    let is_sensitive = prior.as_ref().map(|s| s.is_sensitive).unwrap_or(false);
    let old_value: String = if is_sensitive {
        "[REDACTED]".to_string()
    } else {
        prior.map(|s| s.value).unwrap_or_default()
    };
    let new_value_for_audit: String = if is_sensitive {
        "[REDACTED]".to_string()
    } else {
        form.setting_value.clone()
    };

    // Update the setting
    let update_request = UpdateSettingRequest {
        value: form.setting_value.clone(),
        reason: None,
    };

    match settings_service
        .update_setting(&form.setting_key, update_request, current_user.member.id)
        .await
    {
        Ok(_) => {
            let display_name = form
                .setting_key
                .split('.')
                .last()
                .unwrap_or(&form.setting_key);
            // Log to unified audit_logs with both before and after so
            // the audit page can render the diff inline. (settings_audit
            // also keeps the same data for richer queries; this row is
            // for the unified view.)
            audit_service
                .log(
                    Some(current_user.member.id),
                    "update_setting",
                    "setting",
                    &form.setting_key,
                    Some(&old_value),
                    Some(&new_value_for_audit),
                    None,
                )
                .await;
            admin_settings_page_inner(
                &settings_service,
                &csrf_service,
                &current_user,
                &session_info,
                Some(format!("Updated '{}'", display_name)),
                None,
            )
            .await
        }
        Err(e) => {
            tracing::error!("Failed to update setting {}: {:?}", form.setting_key, e);
            admin_settings_page_inner(
                &settings_service,
                &csrf_service,
                &current_user,
                &session_info,
                None,
                Some(format!("Failed to update setting: {}", e)),
            )
            .await
        }
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

async fn fetch_settings_by_category(
    settings_service: &SettingsService,
) -> Vec<SettingsCategoryInfo> {
    let all_categories = settings_service
        .get_all_settings()
        .await
        .unwrap_or_default();

    let category_meta = [
        (
            "organization",
            "Organization",
            "Basic organization information",
        ),
        (
            "membership",
            "Membership",
            "Membership approval and duration settings",
        ),
        ("payment", "Payment", "Payment amounts and timing"),
        (
            "features",
            "Features",
            "Enable or disable application features",
        ),
        (
            "integrations",
            "Integrations",
            "Third-party service connections",
        ),
        ("audit", "Audit", "Audit log retention"),
        ("auth", "Authentication", "Login policy and access controls"),
        (
            "submissions",
            "Submissions",
            "Member proposal submissions (talk/session call-for-sessions)",
        ),
        (
            "updates",
            "Updates",
            "Update notifications. Enabling the check contacts the public GitHub releases API.",
        ),
        (
            "bot_challenge",
            "Bot Challenge",
            "Captcha on public signup/donate. Set provider to turnstile and add the secret key to enable.",
        ),
    ];

    let mut result = Vec::new();

    for (name, display_name, description) in category_meta {
        if let Some(category) = all_categories.iter().find(|c| c.name == name) {
            let settings: Vec<SettingInfo> = category
                .settings
                .iter()
                .map(|s| setting_to_info(s))
                .collect();

            if !settings.is_empty() {
                result.push(SettingsCategoryInfo {
                    name: name.to_string(),
                    display_name: display_name.to_string(),
                    description: description.to_string(),
                    settings,
                });
            }
        }
    }

    result
}

fn setting_to_info(setting: &AppSetting) -> SettingInfo {
    // Extract display name from key (e.g., "org.name" -> "Name")
    let display_name = setting
        .key
        .split('.')
        .last()
        .unwrap_or(&setting.key)
        .replace('_', " ")
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().chain(chars).collect(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    let value = if setting.is_sensitive {
        String::new() // Don't expose sensitive values
    } else {
        setting.value.clone()
    };

    let is_timezone = setting.key == "org.timezone";
    let timezone_options = if is_timezone {
        // Current value first (so a custom zone stays selectable), then
        // the common list minus any duplicate of the current value.
        std::iter::once(setting.value.as_str())
            .chain(
                COMMON_TIMEZONES
                    .iter()
                    .copied()
                    .filter(|z| *z != setting.value),
            )
            .map(|z| TzOption {
                value: z.to_string(),
                selected: z == setting.value,
            })
            .collect()
    } else {
        Vec::new()
    };

    let is_signup_mode = setting.key == "membership.signup_mode";
    let signup_mode_options = if is_signup_mode {
        ["approval", "payment"]
            .iter()
            .map(|m| TzOption {
                value: (*m).to_string(),
                selected: *m == setting.value,
            })
            .collect()
    } else {
        Vec::new()
    };

    let is_bot_challenge_provider = setting.key == "bot_challenge.provider";
    let bot_challenge_provider_options = if is_bot_challenge_provider {
        ["disabled", "turnstile"]
            .iter()
            .map(|m| TzOption {
                value: (*m).to_string(),
                selected: *m == setting.value,
            })
            .collect()
    } else {
        Vec::new()
    };

    SettingInfo {
        key: setting.key.clone(),
        display_name,
        value,
        value_type: setting.value_type.as_str().to_string(),
        description: setting.description.clone(),
        is_sensitive: setting.is_sensitive,
        is_timezone,
        timezone_options,
        is_signup_mode,
        signup_mode_options,
        is_bot_challenge_provider,
        bot_challenge_provider_options,
    }
}
