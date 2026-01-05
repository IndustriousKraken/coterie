use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
    Extension,
};
use serde::{Deserialize, Serialize};

use crate::{
    api::{state::AppState, middleware::auth::CurrentUser},
    domain::{AppSetting, UpdateSettingRequest, SettingsCategory},
    error::Result,
};

#[derive(Debug, Serialize)]
pub struct SettingsResponse {
    pub categories: Vec<SettingsCategory>,
}

#[derive(Debug, Deserialize)]
pub struct BatchUpdateRequest {
    pub updates: Vec<SettingUpdate>,
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SettingUpdate {
    pub key: String,
    pub value: String,
}

// Get all settings grouped by category
pub async fn list_settings(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<SettingsResponse>> {
    // TODO: Check if user is admin
    
    let categories = state.service_context.settings_service
        .get_all_settings()
        .await?;
    
    // Filter out sensitive values for non-super-admins
    let filtered_categories: Vec<SettingsCategory> = categories
        .into_iter()
        .map(|mut category| {
            category.settings = category.settings
                .into_iter()
                .map(|mut setting| {
                    if setting.is_sensitive {
                        setting.value = "[REDACTED]".to_string();
                    }
                    setting
                })
                .collect();
            category
        })
        .collect();
    
    Ok(Json(SettingsResponse {
        categories: filtered_categories,
    }))
}

// Get settings for a specific category
pub async fn get_category(
    State(state): State<AppState>,
    Path(category): Path<String>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<Vec<AppSetting>>> {
    // TODO: Check if user is admin
    
    let settings = state.service_context.settings_service
        .get_settings_by_category(&category)
        .await?;
    
    // Filter sensitive values
    let filtered: Vec<AppSetting> = settings
        .into_iter()
        .map(|mut setting| {
            if setting.is_sensitive {
                setting.value = "[REDACTED]".to_string();
            }
            setting
        })
        .collect();
    
    Ok(Json(filtered))
}

// Get a specific setting
pub async fn get_setting(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<AppSetting>> {
    // TODO: Check if user is admin
    
    let mut setting = state.service_context.settings_service
        .get_setting(&key)
        .await?;
    
    if setting.is_sensitive {
        setting.value = "[REDACTED]".to_string();
    }
    
    Ok(Json(setting))
}

// Update a specific setting
pub async fn update_setting(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Extension(user): Extension<CurrentUser>,
    Json(request): Json<UpdateSettingRequest>,
) -> Result<Json<AppSetting>> {
    // TODO: Check if user is admin
    
    let updated = state.service_context.settings_service
        .update_setting(&key, request, user.member.id)
        .await?;
    
    Ok(Json(updated))
}

// Batch update multiple settings
pub async fn batch_update(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Json(request): Json<BatchUpdateRequest>,
) -> Result<(StatusCode, Json<Vec<AppSetting>>)> {
    // TODO: Check if user is admin
    
    let mut updated_settings = Vec::new();
    
    for update in request.updates {
        let update_request = UpdateSettingRequest {
            value: update.value,
            reason: request.reason.clone(),
        };
        
        match state.service_context.settings_service
            .update_setting(&update.key, update_request, user.member.id)
            .await
        {
            Ok(setting) => updated_settings.push(setting),
            Err(e) => {
                tracing::error!("Failed to update setting {}: {:?}", update.key, e);
                // Continue with other updates
            }
        }
    }
    
    Ok((StatusCode::OK, Json(updated_settings)))
}

// Get payment configuration
pub async fn get_payment_config(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<crate::domain::PaymentConfig>> {
    let config = state.service_context.settings_service
        .get_payment_config()
        .await?;
    
    Ok(Json(config))
}

// Get membership configuration
pub async fn get_membership_config(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<crate::domain::MembershipConfig>> {
    let config = state.service_context.settings_service
        .get_membership_config()
        .await?;
    
    Ok(Json(config))
}