//! Regression test for the first-run setup wizard's provisioning
//! post-condition.
//!
//! A successful `POST /setup` MUST leave the first admin fully
//! provisioned: `status = Active`, `is_admin = 1`, `bypass_dues = true`.
//! A prior bug swallowed the `Active` promotion error and still returned
//! `success: true`, leaving a `Pending` + `is_admin = 1` row that no
//! middleware tier admits — a permanent org lockout. This test locks the
//! post-condition (and the unchanged happy-path contract) so that can't
//! silently regress.

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use coterie::api::state::AppState;
use tower::ServiceExt;

mod common;
use common::{build_app_state, fresh_pool};

/// Merged router the way `main.rs` builds it, minus the CSRF layer, so
/// `POST /setup` can be driven directly with a JSON body. Mirrors the
/// harness in `setup_redirect_test.rs`.
fn router_full(state: AppState) -> Router {
    let api_app = coterie::api::create_app(state.clone());
    let web_app = coterie::web::create_web_routes(state.clone());
    api_app
        .merge(web_app)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            coterie::api::middleware::setup::require_setup,
        ))
}

#[tokio::test]
async fn setup_provisions_active_admin() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool.clone()).await;
    let app = router_full(state);

    let body = serde_json::json!({
        "org_name": "Test Org",
        "email": "admin@example.com",
        "username": "admin",
        "full_name": "Admin User",
        "password": "WizardPass1",
        "password_confirm": "WizardPass1",
    });
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/setup")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Task 3.2: unchanged happy-path contract — 200, success: true,
    // HX-Redirect + body redirect to /login.
    assert_eq!(resp.status(), StatusCode::OK, "valid setup POST should 200");
    assert_eq!(
        resp.headers()
            .get("HX-Redirect")
            .and_then(|v| v.to_str().ok()),
        Some("/login"),
        "setup should emit HX-Redirect: /login"
    );
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["success"], true, "response must report success: true");
    assert_eq!(
        json["redirect"], "/login",
        "response redirect must be /login"
    );

    // Task 3.1: the created admin row is fully provisioned.
    let row: (String, i64, i64) = sqlx::query_as(
        "SELECT status, is_admin, bypass_dues FROM members WHERE email = ?",
    )
    .bind("admin@example.com")
    .fetch_one(&pool)
    .await
    .expect("select the created admin row");

    assert_eq!(row.0, "Active", "first admin status must be Active");
    assert_eq!(row.1, 1, "first admin is_admin must be 1");
    assert_eq!(row.2, 1, "first admin bypass_dues must be true (1)");
}
