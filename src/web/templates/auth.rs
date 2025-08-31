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
    web::templates::HtmlTemplate,
};

#[derive(Template)]
#[template(path = "auth/login.html")]
pub struct LoginTemplate {
    pub current_user: Option<super::UserInfo>,
    pub is_admin: bool,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    pub remember_me: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub success: bool,
    pub redirect: Option<String>,
    pub error: Option<String>,
}

// GET /auth/login
pub async fn login_page() -> impl IntoResponse {
    let template = LoginTemplate {
        current_user: None,
        is_admin: false,
    };
    HtmlTemplate(template)
}

// POST /auth/login
pub async fn login_handler(
    State(state): State<AppState>,
    Json(credentials): Json<LoginRequest>,
) -> Response {
    // Find member by username or email
    let member = state.service_context.member_repo
        .find_by_username(&credentials.username)
        .await
        .ok()
        .flatten();
    
    let member = if member.is_none() {
        state.service_context.member_repo
            .find_by_email(&credentials.username)
            .await
            .ok()
            .flatten()
    } else {
        member
    };
    
    if let Some(member) = member {
        // Get password hash from database
        let password_hash = crate::auth::get_password_hash(
            &state.service_context.db_pool,
            &member.email
        ).await.ok().flatten();
        
        // Verify password
        let password_valid = if let Some(hash) = password_hash {
            crate::auth::AuthService::verify_password(
                &credentials.password,
                &hash
            ).await.unwrap_or(false)
        } else {
            false
        };
        
        if password_valid {
            // Create session
            let (session, token) = state.service_context.auth_service
                .create_session(
                    member.id,
                    if credentials.remember_me.unwrap_or(false) { 24 * 30 } else { 24 }
                )
                .await
                .unwrap();
            // Create session cookie
            let cookie_value = format!(
                "session={}; HttpOnly; SameSite=Lax; Path=/; Max-Age={}",
                token,
                if credentials.remember_me.unwrap_or(false) { 
                    60 * 60 * 24 * 30 // 30 days
                } else { 
                    60 * 60 * 24 // 24 hours
                }
            );
            
            let mut headers = HeaderMap::new();
            headers.insert(header::SET_COOKIE, cookie_value.parse().unwrap());
            headers.insert("HX-Redirect", "/portal/dashboard".parse().unwrap());
            
            return (StatusCode::OK, headers, Json(LoginResponse {
                success: true,
                redirect: Some("/portal/dashboard".to_string()),
                error: None,
            })).into_response();
        }
    }
    
    // Invalid credentials
    (StatusCode::UNAUTHORIZED, Json(LoginResponse {
        success: false,
        redirect: None,
        error: Some("Invalid username or password".to_string()),
    })).into_response()
}

// POST /auth/logout
pub async fn logout_handler() -> impl IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::SET_COOKIE,
        "session=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0".parse().unwrap()
    );
    headers.insert("HX-Redirect", "/login".parse().unwrap());
    
    (StatusCode::OK, headers)
}