use std::sync::Arc;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::{
    api::state::{self, LoginLimiter},
    auth::{self, AuthService},
    config::Settings,
    error::{AppError, Result},
    service::audit_service::AuditService,
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
    State(auth_service): State<Arc<AuthService>>,
    State(settings): State<Arc<Settings>>,
    State(login_limiter): State<LoginLimiter>,
    State(db_pool): State<SqlitePool>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(req): Json<LoginRequest>,
) -> Result<(CookieJar, Json<LoginResponse>)> {
    // Rate-limit login attempts per IP
    let ip = state::client_ip(&headers, settings.server.trust_forwarded_for());
    if !login_limiter.0.check_and_record(ip) {
        return Err(AppError::TooManyRequests);
    }

    // Get password hash from database
    let password_hash = auth::get_password_hash(&db_pool, &req.email)
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
    let member = auth::get_member_by_email(&db_pool, &req.email)
        .await?
        .ok_or(AppError::Unauthorized)?;

    // Reject login for Pending/Suspended. Expired is allowed through so
    // the member can reach the restoration flow and update payment.
    use crate::domain::MemberStatus;
    match member.status {
        MemberStatus::Active | MemberStatus::Honorary | MemberStatus::Expired => {}
        MemberStatus::Pending | MemberStatus::Suspended => {
            return Err(AppError::Forbidden);
        }
    }

    // Invalidate pre-existing sessions to prevent session fixation.
    let _ = auth_service
        .invalidate_all_sessions(member.id)
        .await;

    // Create session (returns both session and token)
    let (_session, token) = auth_service
        .create_session(member.id, 24)
        .await?;

    // Create cookie with the actual token. The Secure flag tracks whether
    // the deployment is TLS-terminated; see ServerConfig::cookies_are_secure.
    let cookie = auth_service
        .create_session_cookie(&token, settings.server.cookies_are_secure());

    Ok((
        jar.add(cookie),
        Json(LoginResponse {
            message: "Login successful".to_string(),
        })
    ))
}

/// CSRF is enforced at the application root by
/// `csrf_protect_unless_exempt`, so this handler runs only after a
/// valid token has been seen. We don't re-validate here.
pub async fn logout(
    State(auth_service): State<Arc<AuthService>>,
    State(csrf_service): State<Arc<auth::CsrfService>>,
    State(audit_service): State<Arc<AuditService>>,
    jar: CookieJar,
) -> Result<(CookieJar, StatusCode)> {
    if let Some(session_cookie) = jar.get("session") {
        if let Ok(Some(session)) = auth_service
            .validate_session(session_cookie.value())
            .await
        {
            let _ = csrf_service
                .delete_token(&session.id)
                .await;
            audit_service.log(
                Some(session.member_id),
                "logout",
                "session",
                &session.id,
                None,
                None,
                None,
            ).await;
        }
        let _ = auth_service
            .invalidate_session(session_cookie.value())
            .await;
    }

    let jar = jar.add(auth::AuthService::create_logout_cookie());

    Ok((jar, StatusCode::NO_CONTENT))
}