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
/// * **`POST /public/events/:id/register`** — public paid-event
///   registration. The caller is an anonymous visitor (from the
///   marketing site or Coterie's own hosted registration page) with no
///   `session` cookie, so there is no session id to bind a token to.
///   Gated by the CORS allowlist + the per-IP `money_limiter` + the bot
///   challenge, in that order, and it only ever serves `Public`
///   events that have guest registration enabled.
///
/// * **`POST /login`** — the browser portal login form
///   (`templates/auth/login.html`) posts here; no `session` cookie
///   exists yet to bind a CSRF token to. Gated by the per-IP login
///   rate limiter and SameSite=Lax cookies (the same model as
///   `/auth/login`).
///
/// * **`POST /login/totp`** — the second-factor step of the portal
///   login; the caller holds only a `pending_login` cookie at this
///   point, not a `session` cookie, so there's no session id to bind
///   a CSRF token to.
///
/// * **`POST /forgot-password`** — anonymous password-reset request;
///   no session exists. Gated by the per-IP login rate limiter and an
///   enumeration-safe response.
///
/// * **`POST /reset-password`** — anonymous; authorization is the
///   single-use, time-limited reset token carried in the form body,
///   not a session.
///
/// * **`POST /setup`** — the first-run admin-creation wizard; runs
///   before any admin or session exists. Gated by the one-shot "no
///   admin yet" check + `setup_lock`.
///
/// * **`POST /auth/login`** — by definition no session exists yet,
///   so there's nothing to bind a CSRF token to. Login CSRF is a
///   real but separate threat (an attacker forces you to log into
///   their account); it's mitigated via SameSite=Lax cookies and
///   the standard ergonomics of the login form. Adding "anti-login-
///   CSRF" tokens is a future improvement, not part of the
///   state-changing-action CSRF contract this layer enforces.
///
/// * **`POST /auth/login/totp`** — same reason as `/auth/login`: the
///   caller has only a `pending_login` cookie at this point, not a
///   `session` cookie, so there's no session id to bind a CSRF token
///   to. The `pending_login` cookie itself is SameSite=Lax + HttpOnly
///   and lives for 5 minutes; that, the bot rate limit, and the
///   per-member TOTP code requirement are the auth model here.
///
/// `POST /auth/logout` and `POST /logout` are NOT exempt — every
/// authenticated page renders a CSRF meta tag (via `BaseContext`),
/// HTMX stamps the token on every request, and a forced logout is
/// the kind of action that's worth protecting end-to-end.
const CSRF_EXEMPT_PATHS: &[(&str, &str)] = &[
    ("POST", "/api/payments/webhook/stripe"),
    ("POST", "/public/signup"),
    ("POST", "/public/donate"),
    // Public paid-event registration. The caller is an anonymous
    // visitor with no `session` cookie, so there is no session id to
    // bind a CSRF token to. Gated by the CORS allowlist, then the
    // per-IP `money_limiter`, then the bot challenge — in that order —
    // and further constrained to `Public`-visibility events that have
    // guest registration enabled.
    ("POST", "/public/events/:id/register"),
    // Browser portal web-auth forms: the caller has no `session`
    // cookie yet (these endpoints exist to authenticate or first-
    // provision them), so there is no session id to bind a CSRF token
    // to. See the doc comment above for each entry's rationale.
    ("POST", "/login"),
    ("POST", "/login/totp"),
    ("POST", "/forgot-password"),
    ("POST", "/reset-password"),
    ("POST", "/setup"),
    // JSON auth API: same session-less rationale as the web forms.
    ("POST", "/auth/login"),
    ("POST", "/auth/login/totp"),
];

fn is_exempt(method: &Method, path: &str) -> bool {
    CSRF_EXEMPT_PATHS
        .iter()
        .any(|(m, p)| *m == method.as_str() && path_matches(p, path))
}

/// Exact segment match, except a `:param` segment in the pattern matches
/// any one non-empty segment.
///
/// This layer sits above the router, so it compares raw request paths
/// against the route patterns itself rather than reading axum's
/// `MatchedPath` — which isn't populated yet here. String equality alone
/// would silently fail to exempt any parameterized route, and "silently
/// not exempt" on a session-less endpoint means a 403 nobody can debug.
fn path_matches(pattern: &str, path: &str) -> bool {
    let mut pattern_segments = pattern.split('/');
    let mut path_segments = path.split('/');
    loop {
        match (pattern_segments.next(), path_segments.next()) {
            (None, None) => return true,
            (Some(p), Some(actual)) => {
                let ok = if let Some(name) = p.strip_prefix(':') {
                    !name.is_empty() && !actual.is_empty()
                } else {
                    p == actual
                };
                if !ok {
                    return false;
                }
            }
            _ => return false,
        }
    }
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
    let parsed: CsrfField =
        serde_urlencoded::from_bytes(&bytes).map_err(|_| AppError::Forbidden)?;
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
    let stream = futures_util::stream::once(async move { Ok::<_, std::io::Error>(stream_bytes) });
    let mut multipart = multer::Multipart::new(stream, boundary);

    let mut token: Option<String> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| AppError::Forbidden)?
    {
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

#[cfg(test)]
mod path_match_tests {
    use super::*;

    #[test]
    fn parameterized_exempt_path_matches_a_concrete_id() {
        let post = Method::POST;
        assert!(is_exempt(
            &post,
            "/public/events/f7c1e1a0-0000-4000-8000-000000000000/register",
        ));
        // A missing or extra segment is not a match, so the exemption
        // can't leak onto a neighbouring route.
        assert!(!is_exempt(&post, "/public/events//register"));
        assert!(!is_exempt(&post, "/public/events/register"));
        assert!(!is_exempt(&post, "/public/events/abc/register/extra"));
        assert!(!is_exempt(&post, "/public/events/abc/cancel"));
        // Static entries still match exactly, and only for their method.
        assert!(is_exempt(&post, "/public/donate"));
        assert!(!is_exempt(&post, "/public/donate/extra"));
        assert!(!is_exempt(&Method::DELETE, "/public/donate"));
    }
}
