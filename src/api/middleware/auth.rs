use axum::{
    extract::{Request, State},
    http::Uri,
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::CookieJar;

use crate::{
    api::state::AppState,
    domain::{Member, MemberStatus},
    error::AppError,
};

#[derive(Clone)]
pub struct CurrentUser {
    pub member: Member,
}

#[derive(Clone)]
pub struct SessionInfo {
    pub session_id: String,
}

struct AccessPolicy {
    allowed_statuses: &'static [MemberStatus],
    require_admin: bool,
    enforce_admin_totp: bool,
    on_reject: RejectBehavior,
}

#[derive(Clone, Copy)]
enum RejectBehavior {
    Json401,
    RedirectToLogin,
    RedirectToRestoreOrLogin,
    RedirectToDashboardOrLogin,
}

struct Authenticated {
    member: Member,
    session_id: String,
}

enum RejectReason {
    NoCookie,
    InvalidSession,
    MemberNotFound,
    StatusBlocked(MemberStatus),
    NotAdmin,
    AdminTotpMissing,
}

async fn authenticate(
    state: &AppState,
    jar: &CookieJar,
    policy: &AccessPolicy,
) -> Result<Authenticated, RejectReason> {
    let cookie = jar.get("session").ok_or(RejectReason::NoCookie)?;
    let session = state
        .service_context
        .auth_service
        .validate_session(cookie.value())
        .await
        .map_err(|_| RejectReason::InvalidSession)?
        .ok_or(RejectReason::InvalidSession)?;
    let member = state
        .service_context
        .member_repo
        .find_by_id(session.member_id)
        .await
        .map_err(|_| RejectReason::MemberNotFound)?
        .ok_or(RejectReason::MemberNotFound)?;
    if !policy.allowed_statuses.contains(&member.status) {
        return Err(RejectReason::StatusBlocked(member.status.clone()));
    }
    if policy.require_admin && !member.is_admin {
        return Err(RejectReason::NotAdmin);
    }
    if policy.require_admin && policy.enforce_admin_totp {
        // Soft-fail to "not enforced" on setting-lookup error so a
        // setup hiccup never locks every admin out.
        let enforce = state
            .service_context
            .settings_service
            .get_setting("auth.require_totp_for_admins")
            .await
            .ok()
            .map(|s| s.value == "true")
            .unwrap_or(false);
        if enforce
            && !state
                .service_context
                .totp_service
                .is_enabled(member.id)
                .await
                .unwrap_or(false)
        {
            return Err(RejectReason::AdminTotpMissing);
        }
    }
    Ok(Authenticated { member, session_id: session.id })
}

fn redirect_to_login(original_uri: &Uri) -> Response {
    let path = original_uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/portal/dashboard");
    Redirect::to(&format!("/login?redirect={}", urlencoding::encode(path))).into_response()
}

fn render_reject(reason: RejectReason, behavior: RejectBehavior, original_uri: &Uri) -> Response {
    match behavior {
        RejectBehavior::Json401 => match reason {
            RejectReason::StatusBlocked(MemberStatus::Pending) => AppError::Forbidden.into_response(),
            _ => AppError::Unauthorized.into_response(),
        },
        RejectBehavior::RedirectToLogin => redirect_to_login(original_uri),
        RejectBehavior::RedirectToRestoreOrLogin => match reason {
            RejectReason::StatusBlocked(MemberStatus::Expired) => {
                Redirect::to("/portal/restore").into_response()
            }
            _ => redirect_to_login(original_uri),
        },
        RejectBehavior::RedirectToDashboardOrLogin => match reason {
            RejectReason::NotAdmin => Redirect::to("/portal/dashboard").into_response(),
            RejectReason::AdminTotpMissing => {
                Redirect::to("/portal/profile/security?reason=admin_totp_required").into_response()
            }
            _ => redirect_to_login(original_uri),
        },
    }
}

/// Run the shared core and either inject `CurrentUser` + `SessionInfo`
/// onto the request (and forward) or render the per-policy reject
/// response. Used by the redirect-style wrappers.
async fn gate(
    state: &AppState,
    jar: &CookieJar,
    mut request: Request,
    next: Next,
    policy: &AccessPolicy,
) -> Response {
    let original_uri = request.uri().clone();
    match authenticate(state, jar, policy).await {
        Ok(auth) => {
            request.extensions_mut().insert(CurrentUser { member: auth.member });
            request.extensions_mut().insert(SessionInfo { session_id: auth.session_id });
            next.run(request).await
        }
        Err(reason) => render_reject(reason, policy.on_reject, &original_uri),
    }
}

const POLICY_REQUIRE_AUTH: AccessPolicy = AccessPolicy {
    allowed_statuses: &[MemberStatus::Active, MemberStatus::Honorary],
    require_admin: false,
    enforce_admin_totp: false,
    on_reject: RejectBehavior::Json401,
};
const POLICY_REQUIRE_AUTH_REDIRECT: AccessPolicy = AccessPolicy {
    allowed_statuses: &[MemberStatus::Active, MemberStatus::Honorary],
    require_admin: false,
    enforce_admin_totp: false,
    on_reject: RejectBehavior::RedirectToRestoreOrLogin,
};
const POLICY_REQUIRE_RESTORABLE: AccessPolicy = AccessPolicy {
    allowed_statuses: &[MemberStatus::Active, MemberStatus::Honorary, MemberStatus::Expired],
    require_admin: false,
    enforce_admin_totp: false,
    on_reject: RejectBehavior::RedirectToLogin,
};
const POLICY_REQUIRE_ADMIN_REDIRECT: AccessPolicy = AccessPolicy {
    allowed_statuses: &[MemberStatus::Active, MemberStatus::Honorary],
    require_admin: true,
    enforce_admin_totp: true,
    on_reject: RejectBehavior::RedirectToDashboardOrLogin,
};
const POLICY_OPTIONAL_AUTH: AccessPolicy = AccessPolicy {
    allowed_statuses: &[
        MemberStatus::Pending,
        MemberStatus::Active,
        MemberStatus::Expired,
        MemberStatus::Suspended,
        MemberStatus::Honorary,
    ],
    require_admin: false,
    enforce_admin_totp: false,
    on_reject: RejectBehavior::Json401,
};

pub async fn require_auth(
    State(state): State<AppState>,
    jar: CookieJar,
    mut request: Request,
    next: Next,
) -> Result<Response, AppError> {
    match authenticate(&state, &jar, &POLICY_REQUIRE_AUTH).await {
        Ok(auth) => {
            request.extensions_mut().insert(CurrentUser { member: auth.member });
            request.extensions_mut().insert(SessionInfo { session_id: auth.session_id });
            Ok(next.run(request).await)
        }
        Err(RejectReason::StatusBlocked(MemberStatus::Pending)) => Err(AppError::Forbidden),
        Err(_) => Err(AppError::Unauthorized),
    }
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
    request: Request,
    next: Next,
) -> Response {
    gate(&state, &jar, request, next, &POLICY_REQUIRE_AUTH_REDIRECT).await
}

/// Allows Active, Honorary, AND Expired members through. Used on the
/// narrow restoration-flow routes (/portal/restore, payment pages, card
/// management) so Expired members can update payment and restore their
/// account. Active/Honorary members pass through unaffected.
pub async fn require_restorable(
    State(state): State<AppState>,
    jar: CookieJar,
    request: Request,
    next: Next,
) -> Response {
    gate(&state, &jar, request, next, &POLICY_REQUIRE_RESTORABLE).await
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
    request: Request,
    next: Next,
) -> Response {
    gate(&state, &jar, request, next, &POLICY_REQUIRE_ADMIN_REDIRECT).await
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
    if let Ok(auth) = authenticate(&state, &jar, &POLICY_OPTIONAL_AUTH).await {
        request.extensions_mut().insert(CurrentUser { member: auth.member });
    }
    next.run(request).await
}
