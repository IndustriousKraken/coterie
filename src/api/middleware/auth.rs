use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use axum_extra::extract::CookieJar;
use uuid::Uuid;

use crate::{
    api::state::AppState,
    auth::AuthService,
    domain::{Member, MemberStatus},
    error::AppError,
    repository::{MemberRepository, SqliteMemberRepository},
};

#[derive(Clone)]
pub struct CurrentUser {
    pub member: Member,
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

    // Insert current user into request extensions
    request.extensions_mut().insert(CurrentUser { member });

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

    // Insert current user into request extensions
    request.extensions_mut().insert(CurrentUser { member });

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