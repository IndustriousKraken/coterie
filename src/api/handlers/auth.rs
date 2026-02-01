use axum::{
    extract::State,
    http::StatusCode,
    Json,
};
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};

use crate::{
    api::state::AppState,
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
    State(state): State<AppState>,
    jar: CookieJar,
    Json(req): Json<LoginRequest>,
) -> Result<(CookieJar, Json<LoginResponse>)> {
    // Get password hash from database
    let password_hash = auth::get_password_hash(&state.service_context.db_pool, &req.email)
        .await?
        .ok_or(AppError::Unauthorized)?;
    
    // Verify password
    if !auth::AuthService::verify_password(&req.password, &password_hash).await? {
        return Err(AppError::Unauthorized);
    }
    
    // Get member
    let member = auth::get_member_by_email(&state.service_context.db_pool, &req.email)
        .await?
        .ok_or(AppError::Unauthorized)?;
    
    // Create session (returns both session and token)
    let (_session, token) = state.service_context.auth_service
        .create_session(member.id, 24)
        .await?;
    
    // Create cookie with the actual token
    let cookie = state.service_context.auth_service
        .create_session_cookie(&token, false);
    
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