use askama::Template;
use axum::{
    extract::{State, Query},
    response::{IntoResponse, Response, Redirect},
    http::{StatusCode, HeaderMap, header},
    Json,
};
use axum_extra::extract::CookieJar;
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
    pub redirect_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginQuery {
    pub redirect: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    pub remember_me: Option<bool>,
    pub redirect_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub success: bool,
    pub redirect: Option<String>,
    pub error: Option<String>,
}

// GET /login - redirect to dashboard if already logged in
pub async fn login_page(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(query): Query<LoginQuery>,
) -> Response {
    // Check if user already has a valid session
    if let Some(session_cookie) = jar.get("session") {
        if let Ok(Some(_session)) = state.service_context.auth_service
            .validate_session(session_cookie.value())
            .await
        {
            return Redirect::to("/portal/dashboard").into_response();
        }
    }

    let template = LoginTemplate {
        current_user: None,
        is_admin: false,
        redirect_url: query.redirect,
    };
    HtmlTemplate(template).into_response()
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
            let (_session, token) = state.service_context.auth_service
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

            // Use redirect URL if provided, otherwise default to dashboard
            let redirect_url = credentials.redirect_url
                .filter(|url| url.starts_with("/portal"))
                .unwrap_or_else(|| "/portal/dashboard".to_string());

            let mut headers = HeaderMap::new();
            headers.insert(header::SET_COOKIE, cookie_value.parse().unwrap());
            headers.insert("HX-Redirect", redirect_url.parse().unwrap());

            return (StatusCode::OK, headers, Json(LoginResponse {
                success: true,
                redirect: Some(redirect_url),
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

// POST /logout
pub async fn logout_handler(
    State(state): State<AppState>,
    jar: CookieJar,
) -> impl IntoResponse {
    // Properly invalidate session and CSRF token
    if let Some(session_cookie) = jar.get("session") {
        // Get session to find its ID for CSRF token deletion
        if let Ok(Some(session)) = state.service_context.auth_service
            .validate_session(session_cookie.value())
            .await
        {
            // Delete CSRF token for this session
            let _ = state.service_context.csrf_service
                .delete_token(&session.id)
                .await;
        }
        // Invalidate the session
        let _ = state.service_context.auth_service
            .invalidate_session(session_cookie.value())
            .await;
    }

    let mut headers = HeaderMap::new();
    headers.insert(
        header::SET_COOKIE,
        "session=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0".parse().unwrap()
    );
    headers.insert("HX-Redirect", "/login".parse().unwrap());

    (StatusCode::OK, headers)
}