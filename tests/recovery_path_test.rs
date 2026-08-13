//! Integration tests for a47: the recovery path must be REACHABLE when a
//! member is locked out of login, and `POST /reset-password` must tell the
//! truth about whether the password changed.
//!
//! The incident: a member failed five logins, got a correct `429`, then
//! requested a password reset and got `429` again — locked out of the one
//! door built for forgotten passwords. Separately, five reset submissions
//! all returned `200`, so neither the member nor the operator could tell a
//! refusal from a success in any log.
//!
//! Run with: cargo test --test recovery_path_test

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
    Router,
};
use coterie::{
    api::state::AppState,
    domain::{CreateMemberRequest, MemberStatus, UpdateMemberRequest},
};
use sqlx::SqlitePool;
use tower::ServiceExt;
use uuid::Uuid;

mod common;
use common::{build_app_state, fresh_pool, merged_router};

const PASSWORD: &str = "p4ssword_long_enough";
const NEW_PASSWORD: &str = "N3wp4ssword_long_enough";

/// Budget size shared by `login_limiter` and `recovery_limiter`: 5 per 15
/// minutes. Independent buckets, identical size — and since a60, different
/// keys: failed credential attempts per account, recovery requests per IP.
const BUDGET: usize = 5;

async fn harness() -> (SqlitePool, AppState, Router) {
    let pool = fresh_pool().await;
    let state = build_app_state(pool.clone()).await;
    state
        .admin_exists_observed
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let app = merged_router(state.clone());
    (pool, state, app)
}

async fn active_member(state: &AppState, password: &str) -> (Uuid, String) {
    let suffix = Uuid::new_v4();
    let email = format!("u-{suffix}@example.com");
    let member = state
        .service_context
        .member_repo
        .create(CreateMemberRequest {
            email: email.clone(),
            username: format!("u_{}", suffix.simple()),
            full_name: "Test User".to_string(),
            password: password.to_string(),
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

async fn post_login(app: &Router, username: &str, password: &str) -> StatusCode {
    let body = serde_json::json!({ "username": username, "password": password }).to_string();
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

async fn post_form(app: &Router, uri: &str, body: String) -> (StatusCode, String) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

async fn forgot_password(app: &Router, email: &str) -> StatusCode {
    post_form(
        app,
        "/forgot-password",
        format!("email={}", urlencode(email)),
    )
    .await
    .0
}

async fn reset_token(pool: &SqlitePool, member_id: Uuid) -> String {
    coterie::auth::email_tokens::create_password_reset_token(
        pool,
        member_id,
        chrono::Duration::hours(1),
    )
    .await
    .expect("create reset token")
    .token
}

async fn post_reset(app: &Router, token: &str, password: &str) -> (StatusCode, String) {
    post_form(
        app,
        "/reset-password",
        format!(
            "token={}&new_password={}&confirm_password={}",
            urlencode(token),
            urlencode(password),
            urlencode(password)
        ),
    )
    .await
}

// ---------------------------------------------------------------------
// The budgets are independent
// ---------------------------------------------------------------------

/// The regression that trapped a real member on 2026-07-29: five failed
/// logins consumed the credential budget, and the reset request he made
/// next was refused by the same bucket.
#[tokio::test]
async fn a_member_locked_out_of_login_can_still_request_a_reset() {
    let (_pool, state, app) = harness().await;
    let (_id, email) = active_member(&state, PASSWORD).await;

    for _ in 0..BUDGET {
        post_login(&app, &email, "Wr0ngPassword!!").await;
    }
    assert_eq!(
        post_login(&app, &email, "Wr0ngPassword!!").await,
        StatusCode::TOO_MANY_REQUESTS,
        "the credential budget must still close after 5 attempts"
    );

    assert_eq!(
        forgot_password(&app, &email).await,
        StatusCode::OK,
        "recovery runs on its own bucket — a locked-out member must still \
         reach the mechanism built for forgotten passwords"
    );
}

/// Independence runs both ways: burning the recovery budget must not
/// spend anyone's ability to log in.
#[tokio::test]
async fn exhausting_the_recovery_budget_leaves_login_reachable() {
    let (_pool, state, app) = harness().await;
    let (_id, email) = active_member(&state, PASSWORD).await;

    for _ in 0..BUDGET {
        assert_eq!(forgot_password(&app, &email).await, StatusCode::OK);
    }
    assert_eq!(
        forgot_password(&app, &email).await,
        StatusCode::TOO_MANY_REQUESTS,
        "recovery is still limited — it sends email, so it stays abusable"
    );

    assert_eq!(
        post_login(&app, &email, PASSWORD).await,
        StatusCode::OK,
        "the credential budget is untouched by recovery traffic"
    );
}

/// The sharing that a47 explicitly does NOT relax: the second factor stays
/// on the credential budget, so a stolen password can't buy a fresh 5
/// guesses at the 6-digit TOTP space.
///
/// a60 re-keyed that shared budget from the address to the account, so the
/// second-factor attempt has to name an account for the carry-over to mean
/// anything — hence the pending login, minted before the budget is spent
/// (afterwards the first factor itself would be refused).
#[tokio::test]
async fn totp_still_shares_the_credential_budget_with_login() {
    let (_pool, state, app) = harness().await;
    let (member_id, email) = active_member(&state, PASSWORD).await;

    let pending = state
        .service_context
        .pending_login_service
        .create(member_id, false)
        .await
        .expect("create pending");

    for _ in 0..BUDGET {
        post_login(&app, &email, "Wr0ngPassword!!").await;
    }

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/login/totp")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::COOKIE, format!("pending_login={pending}"))
                .body(Body::from(
                    serde_json::json!({ "code": "000000" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "switching to the second-factor surface must not hand out a fresh budget"
    );
}

// ---------------------------------------------------------------------
// The reset status tells the truth
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_reset_with_an_already_consumed_token_is_not_200() {
    let (pool, state, app) = harness().await;
    let (member_id, _email) = active_member(&state, PASSWORD).await;
    let token = reset_token(&pool, member_id).await;

    let (first, _) = post_reset(&app, &token, NEW_PASSWORD).await;
    assert_eq!(first, StatusCode::OK, "the first redemption succeeds");

    let (second, body) = post_reset(&app, &token, NEW_PASSWORD).await;
    assert_ne!(
        second,
        StatusCode::OK,
        "a refused reset must be distinguishable from a successful one in the logs"
    );
    assert!(
        body.contains("invalid or has expired"),
        "the body stays exactly as vague as it was; only the status got honest"
    );
}

#[tokio::test]
async fn a_reset_with_an_over_length_password_is_not_200() {
    let (pool, state, app) = harness().await;
    let (member_id, _email) = active_member(&state, PASSWORD).await;
    let token = reset_token(&pool, member_id).await;

    // 63 characters, 243 bytes — the a46 probe.
    let long = format!("Aa1{}", "\u{1F600}".repeat(60));
    let (status, body) = post_reset(&app, &token, &long).await;

    assert_ne!(status, StatusCode::OK, "a policy refusal is not a success");
    assert!(
        body.contains("Password must be at most 128 bytes (yours is 243)"),
        "the a46 feedback survives the status change; body was: {body}"
    );
}

#[tokio::test]
async fn a_successful_reset_returns_success_and_the_new_password_authenticates() {
    let (pool, state, app) = harness().await;
    let (member_id, email) = active_member(&state, PASSWORD).await;
    let token = reset_token(&pool, member_id).await;

    let (status, body) = post_reset(&app, &token, NEW_PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Your password has been reset"));

    assert_eq!(
        post_login(&app, &email, NEW_PASSWORD).await,
        StatusCode::OK,
        "the reset that reported success must actually have changed the password"
    );
    assert_eq!(
        post_login(&app, &email, PASSWORD).await,
        StatusCode::UNAUTHORIZED,
        "and the old password must no longer work"
    );
}
