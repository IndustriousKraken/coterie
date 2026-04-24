use askama::Template;
use axum::{
    extract::State,
    response::{IntoResponse, Response},
    http::{StatusCode, HeaderMap, header},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::{
    api::state::AppState,
    domain::{CreateMemberRequest, MemberStatus, MembershipType, UpdateMemberRequest},
    web::templates::HtmlTemplate,
};

#[derive(Template)]
#[template(path = "auth/setup.html")]
pub struct SetupTemplate {
    pub current_user: Option<super::UserInfo>,
    pub is_admin: bool,
}

#[derive(Debug, Deserialize)]
pub struct SetupRequest {
    pub org_name: String,
    pub email: String,
    pub username: String,
    pub full_name: String,
    pub password: String,
    pub password_confirm: String,
}

#[derive(Debug, Serialize)]
pub struct SetupResponse {
    pub success: bool,
    pub redirect: Option<String>,
    pub error: Option<String>,
}

// GET /setup
pub async fn setup_page(
    State(state): State<AppState>,
) -> Response {
    // Check if setup is already complete (admin exists)
    let has_admin = check_admin_exists(&state).await;

    if has_admin {
        // Redirect to login if setup already done
        let mut headers = HeaderMap::new();
        headers.insert(header::LOCATION, "/login".parse().unwrap());
        return (StatusCode::SEE_OTHER, headers).into_response();
    }

    let template = SetupTemplate {
        current_user: None,
        is_admin: false,
    };
    HtmlTemplate(template).into_response()
}

// POST /setup
pub async fn setup_handler(
    State(state): State<AppState>,
    Json(request): Json<SetupRequest>,
) -> Response {
    // Validate inputs before acquiring the setup lock so failed requests
    // don't hold the lock while the caller fixes and retries.
    if request.password != request.password_confirm {
        return (StatusCode::BAD_REQUEST, Json(SetupResponse {
            success: false,
            redirect: None,
            error: Some("Passwords do not match".to_string()),
        })).into_response();
    }

    if let Err(msg) = crate::auth::validate_password(&request.password) {
        return (StatusCode::BAD_REQUEST, Json(SetupResponse {
            success: false,
            redirect: None,
            error: Some(msg.to_string()),
        })).into_response();
    }

    if !request.email.contains('@') {
        return (StatusCode::BAD_REQUEST, Json(SetupResponse {
            success: false,
            redirect: None,
            error: Some("Invalid email address".to_string()),
        })).into_response();
    }

    // Serialize first-admin creation. Without this, two concurrent setup
    // requests can both pass the "no admin exists" check and both create
    // admin accounts. The lock is held across check + create + promote.
    let _setup_guard = state.setup_lock.lock().await;

    // Re-check inside the lock using the authoritative is_admin column
    // (not the legacy notes-LIKE heuristic).
    if check_admin_exists(&state).await {
        return (StatusCode::BAD_REQUEST, Json(SetupResponse {
            success: false,
            redirect: None,
            error: Some("Setup has already been completed".to_string()),
        })).into_response();
    }

    // Create the admin member
    let create_request = CreateMemberRequest {
        email: request.email.clone(),
        username: request.username.clone(),
        full_name: request.full_name.clone(),
        password: request.password.clone(),
        membership_type: MembershipType::Lifetime,
    };

    let member = match state.service_context.member_repo.create(create_request).await {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("Failed to create admin user: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(SetupResponse {
                success: false,
                redirect: None,
                error: Some(format!("Failed to create admin user: {}", e)),
            })).into_response();
        }
    };

    // Promote to Active with bypass_dues
    let update_request = UpdateMemberRequest {
        status: Some(MemberStatus::Active),
        bypass_dues: Some(true),
        ..Default::default()
    };

    if let Err(e) = state.service_context.member_repo.update(member.id, update_request).await {
        tracing::error!("Failed to activate admin user: {}", e);
    }

    // Set is_admin = 1 (the authoritative admin flag, used by middleware)
    if let Err(e) = state.service_context.member_repo.set_admin(member.id, true).await {
        tracing::error!("Failed to set is_admin on admin user: {}", e);
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(SetupResponse {
            success: false,
            redirect: None,
            error: Some("Failed to promote user to admin".to_string()),
        })).into_response();
    }

    // Persist the org name to the org.name setting so it shows up in
    // emails, banners, and the public site. Soft-fail: setup itself
    // already succeeded, the admin can edit org.name later if this
    // doesn't take.
    let org_name = request.org_name.trim();
    if !org_name.is_empty() {
        let update = crate::domain::UpdateSettingRequest {
            value: org_name.to_string(),
            reason: Some("Set during initial setup".to_string()),
        };
        if let Err(e) = state.service_context.settings_service
            .update_setting("org.name", update, member.id).await
        {
            tracing::warn!("Couldn't persist org.name during setup ({}); admin can edit later", e);
        }
    }
    tracing::info!("Setup complete for organization: {}", request.org_name);

    let mut headers = HeaderMap::new();
    headers.insert("HX-Redirect", "/login".parse().unwrap());

    (StatusCode::OK, headers, Json(SetupResponse {
        success: true,
        redirect: Some("/login".to_string()),
        error: None,
    })).into_response()
}

/// Check if at least one admin user exists in the database.
/// Uses the `is_admin` column — the authoritative source.
async fn check_admin_exists(state: &AppState) -> bool {
    let result: Result<Option<(i64,)>, _> = sqlx::query_as(
        "SELECT 1 as exists_flag FROM members WHERE is_admin = 1 LIMIT 1",
    )
    .fetch_optional(&state.service_context.db_pool)
    .await;

    match result {
        Ok(Some(_)) => true,
        Ok(None) => false,
        Err(e) => {
            tracing::error!("Failed to check for admin: {}", e);
            false
        }
    }
}
