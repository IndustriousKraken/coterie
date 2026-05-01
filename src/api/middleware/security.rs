//! Top-level security middleware.
//!
//! [`csrf_protect_unless_exempt`] is layered as the OUTERMOST layer on
//! the application router so every state-changing request hits it
//! before any per-route logic runs. The intent is to make CSRF
//! protection *unforgettable*: a new POST/PUT/DELETE/PATCH route
//! inherits protection automatically — you don't have to remember to
//! layer `require_csrf` on the router that carries it.
//!
//! The pre-existing per-route `require_csrf` (in `middleware::auth`)
//! is now redundant and removed from the call graph; the contract
//! lives here and only here.
//!
//! # Why a top-level CSRF layer
//!
//! The previous design layered CSRF per-router (`route_layer`). The
//! portal admin routers correctly opted in; a parallel JSON admin
//! surface (since deleted — see CLAUDE.md and ARCHITECTURE-PUNCHLIST.md)
//! did not. Cookie auth + missing CSRF on admin endpoints meant an
//! admin browsing a malicious page could be made to issue cross-
//! origin POSTs that landed at those endpoints with their session
//! cookie attached.
//!
//! Lifting CSRF to the top of the router inverts the default: every
//! state-changing request is rejected unless it carries a valid
//! token, and adding a new route requires *explicit* opt-out (via
//! [`CSRF_EXEMPT_PATHS`] below) rather than explicit opt-in.

use axum::{
    body::{to_bytes, Body},
    extract::{Request, State},
    http::{header, Method},
    middleware::Next,
    response::Response,
};
use axum_extra::extract::CookieJar;

use crate::{
    api::{middleware::auth::SessionInfo, state::AppState},
    error::AppError,
};

/// Paths that are intentionally exempt from CSRF validation.
///
/// Each entry needs a load-bearing reason. When in doubt, the right
/// answer is to NOT add to this list. PR review on additions should
/// require an explicit "this endpoint cannot carry a session-bound
/// CSRF token because…" justification.
///
/// The current entries:
///
/// * **`POST /api/payments/webhook/stripe`** — Stripe POSTs from its
///   own infrastructure with a `Stripe-Signature` header. The
///   webhook handler verifies the HMAC inside the dispatcher; that
///   IS the auth. No browser involved, no session, no CSRF possible.
///
/// * **`POST /public/signup`** and **`POST /public/donate`** —
///   cross-origin POSTs from the marketing/static site, which has
///   no session and lives on a different origin. These are gated by
///   the CORS allowed-origins list and rate-limited; that's the
///   security model for these endpoints.
///
/// * **`POST /auth/login`** — by definition no session exists yet,
///   so there's nothing to bind a CSRF token to. Login CSRF is a
///   real but separate threat (an attacker forces you to log into
///   their account); it's mitigated via SameSite=Lax cookies and
///   the standard ergonomics of the login form. Adding "anti-login-
///   CSRF" tokens is a future improvement, not part of the
///   state-changing-action CSRF contract this layer enforces.
///
/// * **`POST /auth/logout`** — logging out is idempotent and
///   non-destructive (clears your own session). The cost-benefit of
///   requiring a CSRF token here is poor; an attacker forcing logout
///   is at most a minor nuisance.
const CSRF_EXEMPT_PATHS: &[(&str, &str)] = &[
    ("POST", "/api/payments/webhook/stripe"),
    ("POST", "/public/signup"),
    ("POST", "/public/donate"),
    ("POST", "/auth/login"),
    ("POST", "/auth/logout"),
];

fn is_exempt(method: &Method, path: &str) -> bool {
    CSRF_EXEMPT_PATHS.iter().any(|(m, p)| *m == method.as_str() && *p == path)
}

/// Top-level CSRF middleware.
///
/// Behavior:
///
/// 1. Read-only methods (GET / HEAD / OPTIONS) pass through unmodified.
/// 2. State-changing methods on exempt paths pass through. The
///    handler is responsible for whatever auth scheme replaces CSRF
///    (Stripe signature, CORS gate, etc.).
/// 3. State-changing methods on non-exempt paths: the request must
///    carry a valid session cookie AND a valid `X-CSRF-Token` header
///    (or, for plain `application/x-www-form-urlencoded` bodies, a
///    `csrf_token` form field) bound to that session. Anything else
///    is rejected with 403.
///
/// On success, this middleware injects [`SessionInfo`] into the
/// request extensions so downstream per-route auth middleware doesn't
/// have to re-read the session cookie. (`require_auth` /
/// `require_admin_redirect` still re-validate independently — that's
/// defense in depth, not redundancy worth trimming.)
pub async fn csrf_protect_unless_exempt(
    State(state): State<AppState>,
    jar: CookieJar,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let method = request.method().clone();
    let path = request.uri().path().to_string();

    if matches!(method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return Ok(next.run(request).await);
    }
    if is_exempt(&method, &path) {
        return Ok(next.run(request).await);
    }

    // Need a session to have a CSRF token. No session = blocked.
    let session_cookie = jar.get("session").ok_or(AppError::Forbidden)?;
    let session = state
        .service_context
        .auth_service
        .validate_session(session_cookie.value())
        .await?
        .ok_or(AppError::Forbidden)?;
    let session_id = session.id.clone();

    // Path 1: header-bearing requests (HTMX, fetch). Validate
    // immediately — no need to touch the body.
    if let Some(token) = request
        .headers()
        .get("X-CSRF-Token")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
    {
        let is_valid = state
            .service_context
            .csrf_service
            .validate_token(&session_id, &token)
            .await?;
        if !is_valid {
            return Err(AppError::Forbidden);
        }
        let mut request = request;
        request.extensions_mut().insert(SessionInfo { session_id });
        return Ok(next.run(request).await);
    }

    // Path 2: form-encoded body (urlencoded or multipart). Anything
    // else is rejected — JSON callers must use the header path.
    let content_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    if content_type.starts_with("application/x-www-form-urlencoded") {
        return validate_form_body(state, session_id, request, next).await;
    }
    if content_type.starts_with("multipart/form-data") {
        return validate_multipart_body(state, session_id, &content_type, request, next).await;
    }
    // JSON / missing / other — expected to bring the header.
    Err(AppError::Forbidden)
}

/// Form-urlencoded body path. Buffer body, deserialize the
/// `csrf_token` field, validate, then hand bytes back to the handler.
/// 1MB cap is way above any form we send (largest is a few KB of
/// admin notes).
async fn validate_form_body(
    state: AppState,
    session_id: String,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let (mut parts, body) = request.into_parts();
    let bytes = to_bytes(body, 1024 * 1024)
        .await
        .map_err(|_| AppError::BadRequest("Request body too large".to_string()))?;

    #[derive(serde::Deserialize)]
    struct CsrfField {
        csrf_token: String,
    }
    let parsed: CsrfField = serde_urlencoded::from_bytes(&bytes).map_err(|_| AppError::Forbidden)?;
    let is_valid = state
        .service_context
        .csrf_service
        .validate_token(&session_id, &parsed.csrf_token)
        .await?;
    if !is_valid {
        return Err(AppError::Forbidden);
    }

    parts.extensions.insert(SessionInfo { session_id });
    let request = Request::from_parts(parts, Body::from(bytes));
    Ok(next.run(request).await)
}

/// Multipart body path. The admin event/announcement create+update
/// forms post `multipart/form-data` because they include image
/// uploads. Templates emit `csrf_token` as the first field, so we
/// stream the body through `multer`, stop after we find it, and then
/// reconstruct the request from the buffered bytes for the handler
/// to re-parse. Cap matches the per-image size budget (10MB) plus
/// headroom for other form fields.
///
/// Reaching this code path requires a valid session cookie (checked
/// in the caller), so the buffering DoS surface is admin-only.
async fn validate_multipart_body(
    state: AppState,
    session_id: String,
    content_type: &str,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let boundary = multer::parse_boundary(content_type).map_err(|_| AppError::Forbidden)?;

    let (mut parts, body) = request.into_parts();
    let bytes = to_bytes(body, 12 * 1024 * 1024)
        .await
        .map_err(|_| AppError::BadRequest("Request body too large".to_string()))?;

    // Bytes is reference-counted, so cloning to feed `multer` is cheap.
    let stream_bytes = bytes.clone();
    let stream = futures_util::stream::once(async move {
        Ok::<_, std::io::Error>(stream_bytes)
    });
    let mut multipart = multer::Multipart::new(stream, boundary);

    let mut token: Option<String> = None;
    while let Some(field) = multipart.next_field().await.map_err(|_| AppError::Forbidden)? {
        if field.name() == Some("csrf_token") {
            token = Some(field.text().await.map_err(|_| AppError::Forbidden)?);
            break;
        }
    }
    let token = token.ok_or(AppError::Forbidden)?;

    let is_valid = state
        .service_context
        .csrf_service
        .validate_token(&session_id, &token)
        .await?;
    if !is_valid {
        return Err(AppError::Forbidden);
    }

    parts.extensions.insert(SessionInfo { session_id });
    let request = Request::from_parts(parts, Body::from(bytes));
    Ok(next.run(request).await)
}
