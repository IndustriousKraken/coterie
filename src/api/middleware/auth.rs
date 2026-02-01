use axum::{
    extract::{Request, State},
    middleware::Next,
    response::{Response, Redirect, IntoResponse},
    http::Uri,
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

/// Like require_auth but redirects to login page instead of returning Unauthorized.
/// Used for portal routes where we want a user-friendly redirect.
pub async fn require_auth_redirect(
    State(state): State<AppState>,
    jar: CookieJar,
    mut request: Request,
    next: Next,
) -> Response {
    // Capture the original URI for redirect back after login
    let original_uri = request.uri().clone();

    let session_cookie = match jar.get("session") {
        Some(cookie) => cookie,
        None => return redirect_to_login(&original_uri),
    };

    let auth_service = &state.service_context.auth_service;

    let session = match auth_service.validate_session(session_cookie.value()).await {
        Ok(Some(s)) => s,
        _ => return redirect_to_login(&original_uri),
    };

    // Get member from database
    let member_repo = SqliteMemberRepository::new(state.service_context.db_pool.clone());
    let member = match member_repo.find_by_id(session.member_id).await {
        Ok(Some(m)) => m,
        _ => return redirect_to_login(&original_uri),
    };

    // Check if member is active
    match member.status {
        MemberStatus::Active | MemberStatus::Honorary => {
            // Member is allowed
        }
        _ => return redirect_to_login(&original_uri),
    }

    // Insert current user and session info into request extensions
    request.extensions_mut().insert(CurrentUser { member });
    request.extensions_mut().insert(SessionInfo { session_id: session.id.clone() });

    next.run(request).await
}

fn redirect_to_login(original_uri: &Uri) -> Response {
    let redirect_path = original_uri.path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/portal/dashboard");

    let login_url = format!("/login?redirect={}", urlencoding::encode(redirect_path));
    Redirect::to(&login_url).into_response()
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

/// Middleware that optionally adds session info to requests.
/// Useful for pages that work differently for logged-in vs logged-out users.
#[allow(dead_code)]
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