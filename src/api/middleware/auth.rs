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
///
/// Expired members are redirected to `/portal/restore` (the account
/// restoration flow) rather than `/login` — they need a path to update
/// payment info. Suspended/Pending members shouldn't reach here because
/// the login handler rejects them before a session is created.
pub async fn require_auth_redirect(
    State(state): State<AppState>,
    jar: CookieJar,
    mut request: Request,
    next: Next,
) -> Response {
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

    let member_repo = SqliteMemberRepository::new(state.service_context.db_pool.clone());
    let member = match member_repo.find_by_id(session.member_id).await {
        Ok(Some(m)) => m,
        _ => return redirect_to_login(&original_uri),
    };

    match member.status {
        MemberStatus::Active | MemberStatus::Honorary => {}
        MemberStatus::Expired => {
            // Expired: send them to the restoration flow rather than bouncing
            // to login. The restoration routes use require_restorable and
            // will let them reach the pay-to-restore page.
            return Redirect::to("/portal/restore").into_response();
        }
        _ => return redirect_to_login(&original_uri),
    }

    request.extensions_mut().insert(CurrentUser { member });
    request.extensions_mut().insert(SessionInfo { session_id: session.id.clone() });

    next.run(request).await
}

/// Allows Active, Honorary, AND Expired members through. Used on the
/// narrow restoration-flow routes (/portal/restore, payment pages, card
/// management) so Expired members can update payment and restore their
/// account. Active/Honorary members pass through unaffected.
pub async fn require_restorable(
    State(state): State<AppState>,
    jar: CookieJar,
    mut request: Request,
    next: Next,
) -> Response {
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

    let member_repo = SqliteMemberRepository::new(state.service_context.db_pool.clone());
    let member = match member_repo.find_by_id(session.member_id).await {
        Ok(Some(m)) => m,
        _ => return redirect_to_login(&original_uri),
    };

    match member.status {
        MemberStatus::Active | MemberStatus::Honorary | MemberStatus::Expired => {}
        _ => return redirect_to_login(&original_uri),
    }

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

/// Like require_admin but redirects non-admins to the member dashboard
/// instead of returning a 403 JSON response. Used for portal admin routes.
///
/// Also enforces the optional `auth.require_totp_for_admins` toggle:
/// when set, an admin without `totp_enabled_at` is redirected to the
/// security page rather than the admin route they requested. This
/// gates admin power without breaking their member-side access.
pub async fn require_admin_redirect(
    State(state): State<AppState>,
    jar: CookieJar,
    mut request: Request,
    next: Next,
) -> Response {
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

    let member_repo = SqliteMemberRepository::new(state.service_context.db_pool.clone());
    let member = match member_repo.find_by_id(session.member_id).await {
        Ok(Some(m)) => m,
        _ => return redirect_to_login(&original_uri),
    };

    // Require both Active/Honorary status AND admin flag
    match member.status {
        MemberStatus::Active | MemberStatus::Honorary => {}
        _ => return redirect_to_login(&original_uri),
    }

    if !member.is_admin {
        // Authenticated but not admin: bounce to member dashboard
        return Redirect::to("/portal/dashboard").into_response();
    }

    // Admin-mandatory TOTP enforcement. The setting is read on every
    // admin-route hit; that's a few extra microseconds per request and
    // it lets operators flip the toggle without restart. If the
    // setting lookup fails (e.g. row missing), default to "not
    // enforced" so a setup hiccup never locks every admin out.
    let enforce_admin_totp = state.service_context.settings_service
        .get_setting("auth.require_totp_for_admins").await
        .ok()
        .map(|s| s.value == "true")
        .unwrap_or(false);
    if enforce_admin_totp {
        let enrolled = state.service_context.totp_service
            .is_enabled(member.id).await.unwrap_or(false);
        if !enrolled {
            return Redirect::to("/portal/profile/security?reason=admin_totp_required")
                .into_response();
        }
    }

    request.extensions_mut().insert(CurrentUser { member });
    request.extensions_mut().insert(SessionInfo { session_id: session.id.clone() });

    next.run(request).await
}

// `require_admin` was a middleware for the JSON `/admin/*` and
// `/api/*` admin-only routes. Both surfaces were deleted (admin
// actions live in the portal at `/portal/admin/*`, gated by
// `require_admin_redirect`); the middleware went with them.

// CSRF used to be a per-router middleware here. It now lives at the
// top of the application router as
// `middleware::security::csrf_protect_unless_exempt` so adding a new
// state-changing route can't accidentally skip protection — see
// CLAUDE.md and ARCHITECTURE.md for the rationale.

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