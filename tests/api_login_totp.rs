//! Integration tests for the JSON 2FA login surface: `POST /auth/login`
//! must defer session creation when the member has TOTP enrolled, and
//! `POST /auth/login/totp` must complete the flow against either the
//! `pending_login` cookie or a body-provided `pending_token`.
//!
//! The harness builds the API router (`coterie::api::create_app`)
//! against an in-memory SQLite — the top-level CSRF + setup layers from
//! `main.rs` are deliberately omitted so the tests exercise the auth
//! handlers in isolation. (Both `/auth/login` and `/auth/login/totp`
//! are in `CSRF_EXEMPT_PATHS` in production, so the CSRF layer would
//! pass them through anyway.)
//!
//! Run with: cargo test --test api_login_totp

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

/// Create a member through the repo so password hashing matches the
/// production path, then flip them to Active so the login handler
/// doesn't reject for the default Pending status. Returns the member id
/// and the email used.
async fn make_member(state: &AppState) -> (Uuid, String) {
    use coterie::domain::{CreateMemberRequest, MemberStatus, UpdateMemberRequest};
    let suffix = Uuid::new_v4();
    let email = format!("u-{}@example.com", suffix);
    let member = state
        .service_context
        .member_repo
        .create(CreateMemberRequest {
            email: email.clone(),
            username: format!("u_{}", suffix.simple()),
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
    (member.id, email)
}

/// Run TOTP enrollment end-to-end against `TotpService` and return the
/// `TOTP` instance the test can use to generate fresh codes.
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
    coterie::api::create_app(state)
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

async fn read_json(body: Body) -> Value {
    let bytes = to_bytes(body, 64 * 1024).await.expect("read body");
    if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("parse json")
    }
}

/// Pull all `Set-Cookie` headers into one Vec; tests then inspect by
/// name to ensure session presence / absence as appropriate.
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

/// Get the value of a `Set-Cookie` for `name`. Returns the bare token
/// (no attributes), or `None` if not present / empty / clearing-cookie.
fn cookie_value(set_cookies: &[String], name: &str) -> Option<String> {
    set_cookies.iter().find_map(|c| {
        let prefix = format!("{}=", name);
        if !c.starts_with(&prefix) {
            return None;
        }
        let after = &c[prefix.len()..];
        let value: &str = after.split(';').next().unwrap_or("").trim();
        if value.is_empty() {
            None
        } else {
            Some(value.to_string())
        }
    })
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

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[tokio::test]
async fn json_login_for_totp_enrolled_returns_202_no_session() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool.clone()).await;
    let (member_id, email) = make_member(&state).await;

    let _totp = enroll_totp(
        state.service_context.totp_service.as_ref(),
        member_id,
        &email,
    )
    .await;

    let app = build_app(state.clone());
    let resp = app
        .oneshot(post_json(
            "/auth/login",
            serde_json::json!({
                "email": email,
                "password": PASSWORD,
            }),
            &[],
        ))
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::ACCEPTED,
        "TOTP-enrolled login must yield 202"
    );

    let cookies = collect_set_cookies(resp.headers());
    assert!(
        !has_cookie_named(&cookies, "session"),
        "no `session` cookie may be set at the password-only step; got {:?}",
        cookies
    );
    assert!(
        has_cookie_named(&cookies, "pending_login"),
        "expected a `pending_login` cookie; got {:?}",
        cookies
    );

    let body = read_json(resp.into_body()).await;
    assert_eq!(body["message"], "2fa_required");
    assert!(
        body["pending_token"]
            .as_str()
            .map(|s| !s.is_empty())
            .unwrap_or(false),
        "pending_token must be present and non-empty: {:?}",
        body
    );

    assert_eq!(
        count_sessions(&pool, member_id).await,
        0,
        "no session row may exist after the password-only step"
    );
}

#[tokio::test]
async fn json_login_totp_with_valid_code_creates_session() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool.clone()).await;
    let (member_id, email) = make_member(&state).await;
    let totp = enroll_totp(
        state.service_context.totp_service.as_ref(),
        member_id,
        &email,
    )
    .await;

    let app = build_app(state.clone());

    // Step 1: password-only login → 202 + pending_login cookie.
    let resp = app
        .clone()
        .oneshot(post_json(
            "/auth/login",
            serde_json::json!({
                "email": email,
                "password": PASSWORD,
            }),
            &[],
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let cookies = collect_set_cookies(resp.headers());
    let pending =
        cookie_value(&cookies, "pending_login").expect("pending_login cookie should be set");
    assert_eq!(
        count_pending(&pool, member_id).await,
        1,
        "exactly one pending row should exist after /auth/login"
    );

    // Step 2: submit the TOTP code, carrying the pending cookie.
    let code = totp.generate_current().expect("current code");
    let resp = app
        .oneshot(post_json(
            "/auth/login/totp",
            serde_json::json!({ "code": code }),
            &[format!("pending_login={}", pending)],
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "valid TOTP code should issue a session"
    );

    let cookies = collect_set_cookies(resp.headers());
    assert!(
        has_cookie_named(&cookies, "session"),
        "session cookie must be set on success; got {:?}",
        cookies
    );
    // The handler also clears the pending cookie (empty value, Max-Age=0).
    assert!(
        cookies
            .iter()
            .any(|c| c.starts_with("pending_login=") && c.contains("Max-Age=0")),
        "pending_login cookie should be cleared; got {:?}",
        cookies
    );

    assert_eq!(
        count_pending(&pool, member_id).await,
        0,
        "pending row should be consumed"
    );
    assert_eq!(
        count_sessions(&pool, member_id).await,
        1,
        "exactly one session row should exist after /auth/login/totp"
    );
}

#[tokio::test]
async fn json_login_totp_with_wrong_code_returns_unauthorized() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool.clone()).await;
    let (member_id, email) = make_member(&state).await;
    let totp = enroll_totp(
        state.service_context.totp_service.as_ref(),
        member_id,
        &email,
    )
    .await;

    let app = build_app(state.clone());

    let resp = app
        .clone()
        .oneshot(post_json(
            "/auth/login",
            serde_json::json!({ "email": email, "password": PASSWORD }),
            &[],
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    let cookies = collect_set_cookies(resp.headers());
    let pending =
        cookie_value(&cookies, "pending_login").expect("pending_login cookie should be set");

    // Wrong 6-digit code (TOTP code is generated from `totp`; we
    // pick something that's almost certainly not the current window).
    let real_code = totp.generate_current().expect("current code");
    let wrong_code = if real_code == "000000" {
        "111111"
    } else {
        "000000"
    };

    let resp = app
        .clone()
        .oneshot(post_json(
            "/auth/login/totp",
            serde_json::json!({ "code": wrong_code }),
            &[format!("pending_login={}", pending)],
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "wrong code must yield 401"
    );
    assert_eq!(
        count_sessions(&pool, member_id).await,
        0,
        "no session may be created on a wrong code"
    );

    // The pending row stays so the client can retry until expiry.
    assert_eq!(
        count_pending(&pool, member_id).await,
        1,
        "pending row must survive a wrong-code attempt for retry"
    );

    // Sanity: a subsequent retry with the real code consumes it.
    let resp = app
        .oneshot(post_json(
            "/auth/login/totp",
            serde_json::json!({ "code": real_code }),
            &[format!("pending_login={}", pending)],
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "retry with valid code should succeed"
    );
    assert_eq!(count_sessions(&pool, member_id).await, 1);
    assert_eq!(count_pending(&pool, member_id).await, 0);
}

#[tokio::test]
async fn json_login_no_totp_still_returns_200() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool.clone()).await;
    let (member_id, email) = make_member(&state).await;
    // No TOTP enrollment — the 1-step path must still mint a session.

    let app = build_app(state.clone());
    let resp = app
        .oneshot(post_json(
            "/auth/login",
            serde_json::json!({ "email": email, "password": PASSWORD }),
            &[],
        ))
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "non-TOTP login must remain 200"
    );
    let cookies = collect_set_cookies(resp.headers());
    assert!(
        has_cookie_named(&cookies, "session"),
        "session cookie must be set on a 1-step login; got {:?}",
        cookies
    );
    assert!(
        !has_cookie_named(&cookies, "pending_login"),
        "no pending_login cookie should be set on a 1-step login; got {:?}",
        cookies
    );

    let body = read_json(resp.into_body()).await;
    assert_eq!(body["message"], "Login successful");
    assert_eq!(count_sessions(&pool, member_id).await, 1);
}

/// A failure of `TotpService::is_enabled` during the password-only step
/// MUST NOT silently slip the request through as not-enrolled — that
/// would race a DB blip into a 2FA bypass. The handler must surface a
/// 500, set no `session` cookie, and not mint a `pending_login` row.
#[tokio::test]
async fn json_login_returns_500_if_totp_enrollment_check_errors() {
    let pool = fresh_pool().await;
    let failing_totp = failing_totp_service().await;
    let state = build_app_state_with_totp(pool.clone(), failing_totp).await;
    let (member_id, email) = make_member(&state).await;

    let app = build_app(state.clone());
    let resp = app
        .oneshot(post_json(
            "/auth/login",
            serde_json::json!({ "email": email, "password": PASSWORD }),
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
}

/// `/auth/login/totp` SHALL share the per-IP login budget with
/// `/auth/login`. After the budget is exhausted by repeated wrong-code
/// attempts, the next submission MUST return 429 — preventing a
/// stolen-password attacker from brute-forcing the 6-digit TOTP space.
#[tokio::test]
async fn json_login_totp_returns_429_after_budget_exhausted() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool.clone()).await;
    let (member_id, email) = make_member(&state).await;
    let _totp = enroll_totp(
        state.service_context.totp_service.as_ref(),
        member_id,
        &email,
    )
    .await;

    // Mint the pending row directly via the service so the test starts
    // with a fresh per-IP budget on /auth/login/totp specifically.
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
                "/auth/login/totp",
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
            "/auth/login/totp",
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
