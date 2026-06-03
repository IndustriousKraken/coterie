//! Integration test: `GET /health` reports the embedded build version.
//!
//! The version-string derivation branches are unit-tested directly in
//! `src/version.rs` (they're pure). This test closes the loop on the
//! wiring: the handler actually serializes `crate::version::current()`
//! into the response's `version` field. Under the test build there is
//! no embedded release tag, so `current()` returns the `-dev` fallback
//! — the assertion is against `current()` itself rather than a literal,
//! so it holds whether or not `build.rs` stamped a commit SHA.

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use tower::ServiceExt;

mod common;
use common::{build_app_state, fresh_pool};

#[tokio::test]
async fn health_reports_embedded_version() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool).await;
    let app = coterie::api::create_app(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK, "/health should be 200");

    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("health body is JSON");

    let version = json
        .get("version")
        .and_then(|v| v.as_str())
        .expect("/health response has a string `version` field");

    assert_eq!(
        version,
        coterie::version::current(),
        "/health version must equal crate::version::current()"
    );
}
