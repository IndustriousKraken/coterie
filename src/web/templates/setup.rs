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
    // Check if setup is already complete
    let has_admin = check_admin_exists(&state).await;

    if has_admin {
        return (StatusCode::BAD_REQUEST, Json(SetupResponse {
            success: false,
            redirect: None,
            error: Some("Setup has already been completed".to_string()),
        })).into_response();
    }

    // Validate passwords match
    if request.password != request.password_confirm {
        return (StatusCode::BAD_REQUEST, Json(SetupResponse {
            success: false,
            redirect: None,
            error: Some("Passwords do not match".to_string()),
        })).into_response();
    }

    // Validate password length
    if request.password.len() < 8 {
        return (StatusCode::BAD_REQUEST, Json(SetupResponse {
            success: false,
            redirect: None,
            error: Some("Password must be at least 8 characters".to_string()),
        })).into_response();
    }

    // Validate email format (basic check)
    if !request.email.contains('@') {
        return (StatusCode::BAD_REQUEST, Json(SetupResponse {
            success: false,
            redirect: None,
            error: Some("Invalid email address".to_string()),
        })).into_response();
    }

    // Create the admin member
    let create_request = CreateMemberRequest {
        email: request.email.clone(),
        username: request.username.clone(),
        full_name: request.full_name.clone(),
        password: request.password.clone(),
        membership_type: MembershipType::Lifetime, // Admin gets lifetime membership
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

    // Update to set as active admin with bypass_dues
    let update_request = UpdateMemberRequest {
        status: Some(MemberStatus::Active),
        notes: Some("ADMIN - System administrator".to_string()),
        bypass_dues: Some(true),
        ..Default::default()
    };

    if let Err(e) = state.service_context.member_repo.update(member.id, update_request).await {
        tracing::error!("Failed to update admin user: {}", e);
        // Don't fail here - the user was created, just not fully configured
    }

    // TODO: Store org_name in settings table (for now we just log it)
    tracing::info!("Setup complete for organization: {}", request.org_name);

    // Success - redirect to login
    let mut headers = HeaderMap::new();
    headers.insert("HX-Redirect", "/login".parse().unwrap());

    (StatusCode::OK, headers, Json(SetupResponse {
        success: true,
        redirect: Some("/login".to_string()),
        error: None,
    })).into_response()
}

/// Check if at least one admin user exists in the database
async fn check_admin_exists(state: &AppState) -> bool {
    let result: Result<Option<(i64,)>, _> = sqlx::query_as(
        r#"
        SELECT 1 as exists_flag
        FROM members
        WHERE notes LIKE '%ADMIN%'
        LIMIT 1
        "#,
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
