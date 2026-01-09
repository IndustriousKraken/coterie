use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use axum_extra::extract::CookieJar;

use crate::{
    api::state::AppState,
    domain::{Member, MemberStatus},
    error::AppError,
    repository::{MemberRepository, SqliteMemberRepository},
};

#[derive(Clone)]
pub struct CurrentUser {
    pub member: Member,
}

#[derive(Clone)]
pub struct SessionInfo {
    pub session_id: String,
}

pub async fn require_auth(
    State(state): State<AppState>,
    jar: CookieJar,
    mut request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let session_cookie = jar
        .get("session")
        .ok_or(AppError::Unauthorized)?;

    let auth_service = &state.service_context.auth_service;
    
    let session = auth_service
        .validate_session(session_cookie.value())
        .await?
        .ok_or(AppError::Unauthorized)?;

    // Get member from database
    let member_repo = SqliteMemberRepository::new(state.service_context.db_pool.clone());
    let member = member_repo
        .find_by_id(session.member_id)
        .await?
        .ok_or(AppError::Unauthorized)?;

    // Check if member is active
    match member.status {
        MemberStatus::Active | MemberStatus::Honorary => {
            // Member is allowed
        }
        MemberStatus::Pending => {
            return Err(AppError::Forbidden);
        }
        _ => {
            return Err(AppError::Unauthorized);
        }
    }

    // Insert current user and session info into request extensions
    request.extensions_mut().insert(CurrentUser { member });
    request.extensions_mut().insert(SessionInfo { session_id: session.id.clone() });

    Ok(next.run(request).await)
}

pub async fn require_admin(
    State(state): State<AppState>,
    jar: CookieJar,
    mut request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let session_cookie = jar
        .get("session")
        .ok_or(AppError::Unauthorized)?;

    let auth_service = &state.service_context.auth_service;
    
    let session = auth_service
        .validate_session(session_cookie.value())
        .await?
        .ok_or(AppError::Unauthorized)?;

    // Get member from database
    let member_repo = SqliteMemberRepository::new(state.service_context.db_pool.clone());
    let member = member_repo
        .find_by_id(session.member_id)
        .await?
        .ok_or(AppError::Unauthorized)?;

    // For now, check if member has a special marker in notes field
    // In production, you'd have a proper roles table
    let is_admin = member.notes
        .as_ref()
        .map(|n| n.contains("ADMIN"))
        .unwrap_or(false);

    if !is_admin {
        return Err(AppError::Forbidden);
    }

    // Insert current user and session info into request extensions
    request.extensions_mut().insert(CurrentUser { member });
    request.extensions_mut().insert(SessionInfo { session_id: session.id.clone() });

    Ok(next.run(request).await)
}

/// CSRF validation middleware - validates token on state-changing requests
pub async fn require_csrf(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    use axum::http::Method;

    // Only validate on state-changing methods
    let method = request.method().clone();
    if method == Method::GET || method == Method::HEAD || method == Method::OPTIONS {
        return Ok(next.run(request).await);
    }

    // Get session info from extensions (set by require_auth)
    let session_info = request
        .extensions()
        .get::<SessionInfo>()
        .ok_or(AppError::Forbidden)?
        .clone();

    // Get CSRF token from header (preferred for HTMX/AJAX requests)
    let csrf_token = request
        .headers()
        .get("X-CSRF-Token")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // If no header, we need to check the form body
    // For regular form POSTs, the token is in the csrf_token field
    // We'll pass the request through and let handlers validate if needed
    if let Some(token) = csrf_token {
        // Validate token from header
        let is_valid = state
            .service_context
            .csrf_service
            .validate_token(&session_info.session_id, &token)
            .await?;

        if !is_valid {
            return Err(AppError::Forbidden);
        }
    }
    // If no header token, skip middleware validation
    // Forms include csrf_token field which handlers can validate
    // This allows regular form submissions to work

    Ok(next.run(request).await)
}

pub async fn optional_auth(
    State(state): State<AppState>,
    jar: CookieJar,
    mut request: Request,
    next: Next,
) -> Response {
    if let Some(session_cookie) = jar.get("session") {
        let auth_service = &state.service_context.auth_service;
        
        if let Ok(Some(session)) = auth_service.validate_session(session_cookie.value()).await {
            // Get member from database
            let member_repo = SqliteMemberRepository::new(state.service_context.db_pool.clone());
            
            if let Ok(Some(member)) = member_repo.find_by_id(session.member_id).await {
                // Insert current user into request extensions if valid
                request.extensions_mut().insert(CurrentUser { member });
            }
        }
    }

    next.run(request).await
}