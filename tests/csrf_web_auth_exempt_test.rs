//! Regression test for `fix-csrf-blocks-web-auth-endpoints`.
//!
//! The top-level CSRF layer (`csrf_protect_unless_exempt`) is the
//! OUTERMOST middleware on the merged app. For any state-changing
//! method that is not GET/HEAD/OPTIONS and not in `CSRF_EXEMPT_PATHS`,
//! it requires a `session` cookie before it inspects anything else —
//! returning 403 Forbidden before the handler runs.
//!
//! The browser portal's auth forms POST to the WEB routes (`/login`,
//! `/login/totp`, `/forgot-password`, `/reset-password`, `/setup`),
//! whose callers are session-less by definition. Before the fix those
//! paths were NOT exempt, so the CSRF layer 403'd every login, 2FA,
//! password-reset, and first-run-setup POST before the handler ever
//! ran — locking the portal out of its own auth flows.
//!
//! This test builds the FULL merged app exactly the way `main.rs`
//! does (mirroring `tests/csrf_coverage_test.rs`), then asserts each
//! of the five anonymous web-auth POSTs is NOT rejected with 403 by
//! the CSRF layer, while a genuinely protected portal route still is.

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use tower::ServiceExt;

mod common;
use common::{build_app_state, fresh_pool};

/// Build the full merged app the way `main.rs` does: merge the API and
/// web routers, then layer the setup-check and CSRF middleware
/// (CSRF outermost, so it runs first). A unit test of the middleware
/// in isolation can't prove this — the exempt check only matters once
/// the web routes are actually behind the top-level CSRF layer.
async fn build_app() -> Router {
    let pool = fresh_pool().await;
    let app_state = build_app_state(pool).await;

    let api_app = coterie::api::create_app(app_state.clone());
    let web_app = coterie::web::create_web_routes(app_state.clone());

    api_app
        .merge(web_app)
        .layer(axum::middleware::from_fn_with_state(
            app_state.clone(),
            coterie::api::middleware::setup::require_setup,
        ))
        .layer(axum::middleware::from_fn_with_state(
            app_state,
            coterie::api::middleware::security::csrf_protect_unless_exempt,
        ))
}

#[tokio::test]
async fn anonymous_web_auth_posts_are_not_csrf_rejected() {
    // The five browser-facing web-auth POSTs. Their callers hold no
    // `session` cookie yet (these endpoints exist to authenticate or
    // first-provision the caller), so they cannot carry a session-
    // bound CSRF token and must be exempt from the top-level layer.
    let paths = [
        "/login",
        "/login/totp",
        "/forgot-password",
        "/reset-password",
        "/setup",
    ];

    for path in paths {
        let app = build_app().await;

        // No `session` cookie. A well-formed form body for the
        // handler's content type; the body is irrelevant to the CSRF
        // exempt check, which short-circuits before the body is read.
        let req = Request::builder()
            .method("POST")
            .uri(path)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from("username=nobody&password=x"))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();

        // A 403 here is the CSRF-layer rejection this change fixes:
        // the layer demands a session cookie before the handler runs.
        // Any OTHER status proves the request reached its handler (or
        // the setup redirect), i.e. the path is now correctly exempt.
        assert_ne!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "anonymous POST {path} must pass the top-level CSRF layer \
             (be exempt), not be rejected with 403; got {}",
            resp.status()
        );
    }
}

#[tokio::test]
async fn protected_portal_post_still_csrf_rejected() {
    // Counter-assertion: the exempt list was widened precisely, not
    // CSRF globally disabled. A state-changing portal admin route with
    // no session SHALL still be rejected by the CSRF layer with 403.
    let app = build_app().await;

    let req = Request::builder()
        .method("POST")
        .uri("/portal/admin/members/00000000-0000-0000-0000-000000000000/update")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "POST /portal/admin/members/.../update should still be rejected \
         by the top-level CSRF layer with 403; got {}",
        resp.status()
    );
}
