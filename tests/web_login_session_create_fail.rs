//! Integration test for `POST /login`'s session-create failure path:
//! a successful password verify followed by an `AuthService::create_session`
//! error must surface as `500 Internal Server Error` — never a panic —
//! and must leave no `session` cookie on the response.
//!
//! The harness injects an `AuthService` whose backing pool has been
//! closed; the rest of the services keep the live pool so member lookup
//! and password verify still work, isolating the failure to the session
//! INSERT.
//!
//! Run with: cargo test --test web_login_session_create_fail

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
    Router,
};
use coterie::api::state::AppState;
use serde_json::Value;
use sqlx::SqlitePool;
use tower::ServiceExt;
use uuid::Uuid;

mod common;
use common::{build_app_state_with_auth, failing_auth_service, fresh_pool};

const PASSWORD: &str = "p4ssword_long_enough";

async fn make_active_member(state: &AppState) -> (Uuid, String) {
    use coterie::domain::{CreateMemberRequest, MemberStatus, UpdateMemberRequest};
    let suffix = Uuid::new_v4();
    let email = format!("u-{}@example.com", suffix);
    let username = format!("u_{}", suffix.simple());
    let member = state
        .service_context
        .member_repo
        .create(CreateMemberRequest {
            email: email.clone(),
            username: username.clone(),
            full_name: "Test User".to_string(),
            password: PASSWORD.to_string(),
            membership_type_id: None,
            ..Default::default()
        })
        .await
        .expect("create member");
    state
        .service_context
        .member_repo
        .update(
            member.id,
            UpdateMemberRequest {
                status: Some(MemberStatus::Active),
                ..Default::default()
            },
        )
        .await
        .expect("activate member");
    (member.id, username)
}

fn build_app(state: AppState) -> Router {
    coterie::web::create_web_routes(state)
}

fn post_json(uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

fn collect_set_cookies(resp_headers: &axum::http::HeaderMap) -> Vec<String> {
    resp_headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(|s| s.to_string()))
        .collect()
}

fn has_cookie_named(set_cookies: &[String], name: &str) -> bool {
    set_cookies
        .iter()
        .any(|c| c.starts_with(&format!("{}=", name)))
}

async fn read_body_text(body: Body) -> String {
    let bytes = to_bytes(body, 64 * 1024).await.expect("read body");
    String::from_utf8(bytes.to_vec()).unwrap_or_default()
}

async fn count_sessions(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sessions")
        .fetch_one(pool)
        .await
        .expect("count sessions")
}

/// A failure of `AuthService::create_session` during the password-only
/// step MUST NOT panic the handler — that drops the connection and
/// leaves no actionable trail. Surface as a clean 500 with the generic
/// "Login failed. Please try again." body and no `session` cookie.
#[tokio::test]
async fn login_handler_returns_500_when_session_create_fails() {
    let pool = fresh_pool().await;
    let failing_auth = failing_auth_service().await;
    let state = build_app_state_with_auth(pool.clone(), failing_auth).await;
    let (_member_id, username) = make_active_member(&state).await;

    let app = build_app(state);
    let resp = app
        .oneshot(post_json(
            "/login",
            serde_json::json!({
                "username": username,
                "password": PASSWORD,
            }),
        ))
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "session-create failure must surface as 500, not a panic or success"
    );

    let cookies = collect_set_cookies(resp.headers());
    assert!(
        !has_cookie_named(&cookies, "session"),
        "no `session` cookie may be set when create_session fails; got {:?}",
        cookies,
    );

    let body = read_body_text(resp.into_body()).await;
    let parsed: Value = serde_json::from_str(&body).expect("body should be JSON");
    assert_eq!(
        parsed.get("success").and_then(Value::as_bool),
        Some(false),
        "LoginResponse.success must be false; body was {body}"
    );
    assert!(
        parsed
            .get("error")
            .and_then(Value::as_str)
            .map(|s| !s.is_empty())
            .unwrap_or(false),
        "LoginResponse.error must be Some(non-empty); body was {body}"
    );
    assert!(
        parsed.get("redirect").map(Value::is_null).unwrap_or(false),
        "LoginResponse.redirect must be null on failure; body was {body}"
    );

    // The failure path used the closed auth-service pool, so the live
    // pool's `sessions` table must remain empty — no row written, no
    // half-committed state.
    assert_eq!(
        count_sessions(&pool).await,
        0,
        "no session row may be written when create_session fails"
    );
}
