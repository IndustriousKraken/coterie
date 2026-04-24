use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};

use crate::{
    api::state::{self, AppState},
    auth,
    error::{AppError, Result},
};

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub message: String,
}

pub async fn login(
    State(app): State<AppState>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(req): Json<LoginRequest>,
) -> Result<(CookieJar, Json<LoginResponse>)> {
    // Rate-limit login attempts per IP
    let ip = state::client_ip(&headers, app.settings.server.trust_forwarded_for());
    if !app.login_limiter.check_and_record(ip) {
        return Err(AppError::TooManyRequests);
    }

    // Get password hash from database
    let password_hash = auth::get_password_hash(&app.service_context.db_pool, &req.email)
        .await?;

    let password_hash = match password_hash {
        Some(h) => h,
        None => {
            // User not found — burn Argon2 time to prevent timing-based enumeration.
            auth::AuthService::verify_dummy(&req.password).await;
            return Err(AppError::Unauthorized);
        }
    };

    // Verify password
    if !auth::AuthService::verify_password(&req.password, &password_hash).await? {
        return Err(AppError::Unauthorized);
    }

    // Get member
    let member = auth::get_member_by_email(&app.service_context.db_pool, &req.email)
        .await?
        .ok_or(AppError::Unauthorized)?;

    // Invalidate pre-existing sessions to prevent session fixation.
    let _ = app.service_context.auth_service
        .invalidate_all_sessions(member.id)
        .await;

    // Create session (returns both session and token)
    let (_session, token) = app.service_context.auth_service
        .create_session(member.id, 24)
        .await?;

    // Create cookie with the actual token. The Secure flag tracks whether
    // the deployment is TLS-terminated; see ServerConfig::cookies_are_secure.
    let cookie = app.service_context.auth_service
        .create_session_cookie(&token, app.settings.server.cookies_are_secure());
    
    Ok((
        jar.add(cookie),
        Json(LoginResponse {
            message: "Login successful".to_string(),
        })
    ))
}

pub async fn logout(
    State(state): State<AppState>,
    jar: CookieJar,
) -> Result<(CookieJar, StatusCode)> {
    // Get session from cookie
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
        // Invalidate session in database
        let _ = state.service_context.auth_service
            .invalidate_session(session_cookie.value())
            .await;
    }

    // Remove cookie
    let jar = jar.add(auth::AuthService::create_logout_cookie());

    Ok((jar, StatusCode::NO_CONTENT))
}