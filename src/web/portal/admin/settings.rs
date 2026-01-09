use askama::Template;
use axum::{
    extract::State,
    response::{IntoResponse, Response},
    Extension,
    Form,
};
use serde::Deserialize;

use crate::{
    api::{
        middleware::auth::{CurrentUser, SessionInfo},
        state::AppState,
    },
    domain::{AppSetting, UpdateSettingRequest},
    web::templates::{HtmlTemplate, UserInfo},
};
use crate::web::portal::is_admin;

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
}

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
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub csrf_token: String,
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
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> impl IntoResponse {
    admin_settings_page_inner(state, current_user, session_info, None, None).await
}

async fn admin_settings_page_inner(
    state: AppState,
    current_user: CurrentUser,
    session_info: SessionInfo,
    success_message: Option<String>,
    error_message: Option<String>,
) -> Response {
    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let csrf_token = state.service_context.csrf_service
        .generate_token(&session_info.session_id)
        .await
        .unwrap_or_else(|_| "error".to_string());

    let categories = fetch_settings_by_category(&state).await;

    HtmlTemplate(AdminSettingsTemplate {
        current_user: Some(user_info),
        is_admin: true,
        csrf_token,
        categories,
        success_message,
        error_message,
    }).into_response()
}

pub async fn admin_update_setting(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Form(form): Form<UpdateSettingForm>,
) -> impl IntoResponse {
    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

    // Validate CSRF
    let csrf_valid = state.service_context.csrf_service
        .validate_token(&session_info.session_id, &form.csrf_token)
        .await
        .unwrap_or(false);

    if !csrf_valid {
        return admin_settings_page_inner(
            state,
            current_user,
            session_info,
            None,
            Some("Invalid CSRF token. Please try again.".to_string()),
        ).await;
    }

    // Update the setting
    let update_request = UpdateSettingRequest {
        value: form.setting_value.clone(),
        reason: None,
    };

    match state.service_context.settings_service
        .update_setting(&form.setting_key, update_request, current_user.member.id)
        .await
    {
        Ok(_) => {
            let display_name = form.setting_key.split('.').last().unwrap_or(&form.setting_key);
            admin_settings_page_inner(
                state,
                current_user,
                session_info,
                Some(format!("Updated '{}'", display_name)),
                None,
            ).await
        }
        Err(e) => {
            tracing::error!("Failed to update setting {}: {:?}", form.setting_key, e);
            admin_settings_page_inner(
                state,
                current_user,
                session_info,
                None,
                Some(format!("Failed to update setting: {}", e)),
            ).await
        }
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

async fn fetch_settings_by_category(state: &AppState) -> Vec<SettingsCategoryInfo> {
    let all_categories = state.service_context.settings_service
        .get_all_settings()
        .await
        .unwrap_or_default();

    let category_meta = [
        ("organization", "Organization", "Basic organization information"),
        ("membership", "Membership", "Membership approval and duration settings"),
        ("payment", "Payment", "Payment amounts and timing"),
        ("features", "Features", "Enable or disable application features"),
        ("integrations", "Integrations", "Third-party service connections"),
    ];

    let mut result = Vec::new();

    for (name, display_name, description) in category_meta {
        if let Some(category) = all_categories.iter().find(|c| c.name == name) {
            let settings: Vec<SettingInfo> = category.settings.iter()
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
    let display_name = setting.key
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

    SettingInfo {
        key: setting.key.clone(),
        display_name,
        value,
        value_type: setting.value_type.as_str().to_string(),
        description: setting.description.clone(),
        is_sensitive: setting.is_sensitive,
    }
}
