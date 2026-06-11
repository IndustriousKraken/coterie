//! Integration tests for the web 2FA login surface: `POST /login` must
//! defer session creation when the member has TOTP enrolled and must
//! fail closed on an enrollment-query error, and `POST /login/totp`
//! must share the per-IP rate-limit budget with `/login`.
//!
//! The harness builds the web router (`coterie::web::create_web_routes`)
//! against an in-memory SQLite — top-level CSRF + setup layers from
//! `main.rs` are deliberately omitted so the tests exercise the auth
//! handlers in isolation, matching `tests/api_login_totp.rs`.
//!
//! Run with: cargo test --test web_login_totp

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
    Router,
};
use coterie::{api::state::AppState, auth::TotpService};
use serde_json::Value;
use sqlx::SqlitePool;
use totp_rs::{Algorithm, TOTP};
use tower::ServiceExt;
use uuid::Uuid;

mod common;
use common::{build_app_state, build_app_state_with_totp, failing_totp_service, fresh_pool};

// ---------------------------------------------------------------------
// Harness helpers
// ---------------------------------------------------------------------

const PASSWORD: &str = "p4ssword_long_enough";

async fn make_member(state: &AppState) -> (Uuid, String, String) {
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
    (member.id, email, username)
}

async fn enroll_totp(svc: &TotpService, member_id: Uuid, email: &str) -> TOTP {
    let init = svc.begin_enrollment(email).expect("begin enrollment");
    let totp = totp_from_b32(&init.secret_base32, email);
    let code = totp.generate_current().expect("generate current");
    let ok = svc
        .confirm_enrollment(member_id, &init.secret_base32, &code, email)
        .await
        .expect("confirm enrollment");
    assert!(ok, "confirm_enrollment must succeed with a fresh code");
    totp
}

fn totp_from_b32(secret_b32: &str, account_name: &str) -> TOTP {
    use totp_rs::Secret;
    let bytes = Secret::Encoded(secret_b32.to_string())
        .to_bytes()
        .expect("decode base32");
    TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        bytes,
        Some("Coterie".to_string()),
        account_name.to_string(),
    )
    .expect("build TOTP")
}

fn build_app(state: AppState) -> Router {
    coterie::web::create_web_routes(state)
}

fn post_json(uri: &str, body: Value, cookies: &[String]) -> Request<Body> {
    let mut b = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if !cookies.is_empty() {
        b = b.header(header::COOKIE, cookies.join("; "));
    }
    b.body(Body::from(serde_json::to_vec(&body).unwrap()))
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

async fn count_sessions(pool: &SqlitePool, member_id: Uuid) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sessions WHERE member_id = ?")
        .bind(member_id.to_string())
        .fetch_one(pool)
        .await
        .expect("count sessions")
}

async fn count_pending(pool: &SqlitePool, member_id: Uuid) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pending_logins WHERE member_id = ?")
        .bind(member_id.to_string())
        .fetch_one(pool)
        .await
        .expect("count pending")
}

async fn read_body_text(body: Body) -> String {
    let bytes = to_bytes(body, 64 * 1024).await.expect("read body");
    String::from_utf8(bytes.to_vec()).unwrap_or_default()
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

/// A failure of `TotpService::is_enabled` during the password-only step
/// MUST NOT silently slip the request through as not-enrolled — that
/// would race a DB blip into a 2FA bypass. The web handler must surface
/// a 500, set no `session` cookie, and not mint a `pending_login` row.
#[tokio::test]
async fn web_login_returns_500_if_totp_enrollment_check_errors() {
    let pool = fresh_pool().await;
    let failing_totp = failing_totp_service().await;
    let state = build_app_state_with_totp(pool.clone(), failing_totp).await;
    let (member_id, _email, username) = make_member(&state).await;

    let app = build_app(state.clone());
    let resp = app
        .oneshot(post_json(
            "/login",
            serde_json::json!({
                "username": username,
                "password": PASSWORD,
            }),
            &[],
        ))
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "TOTP enrollment-check failure must fail closed as 500"
    );

    let cookies = collect_set_cookies(resp.headers());
    assert!(
        !has_cookie_named(&cookies, "session"),
        "no `session` cookie may be set when the enrollment check errors; got {:?}",
        cookies,
    );
    assert!(
        !has_cookie_named(&cookies, "pending_login"),
        "no `pending_login` cookie may be set when the enrollment check errors; got {:?}",
        cookies,
    );
    assert_eq!(
        count_sessions(&pool, member_id).await,
        0,
        "no session row may be created when the enrollment check errors"
    );
    assert_eq!(
        count_pending(&pool, member_id).await,
        0,
        "no pending_login row may be created when the enrollment check errors"
    );
    let body = read_body_text(resp.into_body()).await;
    // sanity: the 500 surface mentions the generic failure message
    assert!(
        body.contains("Login failed"),
        "body should hint at failure: {body}"
    );
}

/// `/login/totp` SHALL share the per-IP login budget with `/login`.
/// After the budget is exhausted by repeated wrong-code attempts, the
/// next submission MUST return 429 — preventing a stolen-password
/// attacker from brute-forcing the 6-digit TOTP space.
#[tokio::test]
async fn web_login_totp_returns_429_after_budget_exhausted() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool.clone()).await;
    let (member_id, email, _username) = make_member(&state).await;
    let _totp = enroll_totp(
        state.service_context.totp_service.as_ref(),
        member_id,
        &email,
    )
    .await;

    // Mint the pending row directly via the service so the test starts
    // with a fresh per-IP budget on /login/totp specifically.
    let pending = state
        .service_context
        .pending_login_service
        .create(member_id, false)
        .await
        .expect("create pending");

    let app = build_app(state.clone());
    let wrong = "000000";
    let cookies = [format!("pending_login={}", pending)];

    // Budget is 5/15min per IP. 5 wrong-code attempts consume the
    // budget while returning 401 on auth failure. The 6th attempt is
    // gated at the limiter and returns 429 without ever reaching the
    // TOTP verify path.
    for i in 1..=5 {
        let resp = app
            .clone()
            .oneshot(post_json(
                "/login/totp",
                serde_json::json!({ "code": wrong }),
                &cookies,
            ))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "attempt {} should be 401",
            i
        );
    }

    let resp = app
        .oneshot(post_json(
            "/login/totp",
            serde_json::json!({ "code": wrong }),
            &cookies,
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "6th attempt must be 429"
    );

    assert_eq!(
        count_sessions(&pool, member_id).await,
        0,
        "no session may be created across the budget-exhausted run"
    );
}
