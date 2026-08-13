//! Integration tests for a60: the credential budget counts **failures**
//! and keys on the **account**, not the address.
//!
//! The incident: on 2026-08-13 an administrator was locked out of
//! production by five consecutive *successful* logins — one of them a
//! correct second factor, two of them a single double-submitted form.
//! Every attempt was recorded before it was authenticated, so doing
//! everything right spent the budget for defending against people doing
//! it wrong.
//!
//! The second defect had a wider blast radius than the reported one: a
//! per-address key meant a handful of mistyped passwords at an event
//! venue denied login to everyone behind that NAT address, including
//! members who had attempted nothing.
//!
//! Run with: cargo test --features test-utils --test credential_rate_limit_test

use std::time::Instant;

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
    Router,
};
use coterie::{
    api::state::AppState,
    domain::{CreateMemberRequest, MemberStatus, UpdateMemberRequest},
};
use tower::ServiceExt;
use uuid::Uuid;

mod common;
use common::{build_app_state, fresh_pool, merged_router};

const PASSWORD: &str = "p4ssword_long_enough";
const WRONG: &str = "Wr0ngPassword!!";

/// Failures allowed per account per 15 minutes.
const ACCOUNT_BUDGET: usize = 5;
/// Distinct accounts one address may fail against per 15 minutes.
const ADDRESS_BREADTH: usize = 20;

async fn harness() -> (AppState, Router) {
    let pool = fresh_pool().await;
    let state = build_app_state(pool).await;
    state
        .admin_exists_observed
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let app = merged_router(state.clone());
    (state, app)
}

async fn active_member(state: &AppState) -> (Uuid, String, String) {
    let suffix = Uuid::new_v4();
    let email = format!("u-{suffix}@example.com");
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

/// Every request in this file comes from the same address: the test
/// settings leave `trust_forwarded_for` off, so all of them land in the
/// 127.0.0.1 bucket. That is exactly the shared-address case the venue
/// scenarios need.
async fn post_login(app: &Router, username: &str, password: &str) -> (StatusCode, String) {
    let body = serde_json::json!({ "username": username, "password": password }).to_string();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

async fn post_totp(app: &Router, code: &str, pending: &str) -> StatusCode {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/login/totp")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::COOKIE, format!("pending_login={pending}"))
                .body(Body::from(serde_json::json!({ "code": code }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

// ---------------------------------------------------------------------
// Only failures count
// ---------------------------------------------------------------------

/// The reported defect, asserted directly: successful logins — including
/// a correct second factor and the same form submitted twice — cost
/// nothing, so no number of them can lock anyone out.
#[tokio::test]
async fn a_run_of_successful_logins_is_never_rejected() {
    let (state, app) = harness().await;
    let (member_id, email, _username) = active_member(&state).await;

    for round in 0..(ACCOUNT_BUDGET * 2) {
        let (status, _) = post_login(&app, &email, PASSWORD).await;
        assert_eq!(status, StatusCode::OK, "login {round} must succeed");
    }

    // The same submission twice: the double-submit that put two rows in
    // the incident log for one login. No de-duplication exists (or is
    // needed) — both succeeded, so both cost nothing.
    assert_eq!(post_login(&app, &email, PASSWORD).await.0, StatusCode::OK);
    assert_eq!(
        post_login(&app, &email, PASSWORD).await.0,
        StatusCode::OK,
        "the double submit is free too"
    );

    // A correct second factor, which under the old limiter made using
    // MFA correctly exhaust the budget twice as fast. Any completed
    // second factor would do; the recovery-code path is the one that
    // needs no clock.
    let pending = state
        .service_context
        .pending_login_service
        .create(member_id, false)
        .await
        .expect("create pending");
    let code =
        coterie::auth::recovery_codes::issue_for_member(&state.service_context.db_pool, member_id)
            .await
            .expect("mint recovery codes")
            .remove(0);
    assert_eq!(
        post_totp(&app, &code, &pending).await,
        StatusCode::OK,
        "a correct second factor must succeed and cost nothing"
    );

    let (status, _) = post_login(&app, &email, PASSWORD).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "no budget was ever consumed, so nothing can be exhausted"
    );
}

#[tokio::test]
async fn the_sixth_failure_against_one_account_is_rejected() {
    let (state, app) = harness().await;
    let (_id, email, _username) = active_member(&state).await;

    for attempt in 1..=ACCOUNT_BUDGET {
        let (status, _) = post_login(&app, &email, WRONG).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "attempt {attempt} is 401");
    }
    let (status, _) = post_login(&app, &email, WRONG).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);

    // And the budget stays closed for the correct password too — that is
    // the accepted cost of keying on the account.
    let (status, _) = post_login(&app, &email, PASSWORD).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

/// An over-limit attempt must not reach Argon2. Losing that ordering
/// turns the limiter into a hashing-DoS amplifier and is invisible from
/// the status code alone — the rejected request is compared against a
/// verifying one measured in the same process, so the assertion is a
/// ratio rather than a wall-clock constant.
#[tokio::test]
async fn an_over_limit_attempt_does_no_password_verification() {
    let (state, app) = harness().await;
    let (_id, email, _username) = active_member(&state).await;

    let mut verifying = std::time::Duration::MAX;
    for _ in 0..ACCOUNT_BUDGET {
        let start = Instant::now();
        let (status, _) = post_login(&app, &email, WRONG).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        verifying = verifying.min(start.elapsed());
    }

    let start = Instant::now();
    let (status, _) = post_login(&app, &email, WRONG).await;
    let rejected = start.elapsed();
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);

    assert!(
        rejected * 4 < verifying,
        "the rejected attempt took {rejected:?} against {verifying:?} for one that \
         hashes — it is doing the Argon2 work the budget exists to bound"
    );
}

// ---------------------------------------------------------------------
// The key moved to the account
// ---------------------------------------------------------------------

/// The venue case, and the reason the key moved: one member's failures
/// are between them and their own account.
#[tokio::test]
async fn one_member_locking_themselves_out_does_not_lock_out_the_room() {
    let (state, app) = harness().await;
    let (_id, unlucky, _u1) = active_member(&state).await;
    let (_id, neighbour, _u2) = active_member(&state).await;

    for _ in 0..ACCOUNT_BUDGET {
        post_login(&app, &unlucky, WRONG).await;
    }
    let (status, _) = post_login(&app, &unlucky, WRONG).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);

    let (status, _) = post_login(&app, &neighbour, PASSWORD).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a member who shares only an address with them must still get in"
    );
}

/// Several members at one address fumbling — including one who spends
/// their whole budget — is ordinary traffic and must not reach the
/// address allowance.
#[tokio::test]
async fn a_venue_full_of_members_fumbling_does_not_trip_the_address_budget() {
    let (state, app) = harness().await;
    let (_id, unlucky, _u) = active_member(&state).await;
    let mut others = Vec::new();
    for _ in 0..4 {
        let (_id, email, _u) = active_member(&state).await;
        others.push(email);
    }

    for _ in 0..ACCOUNT_BUDGET {
        post_login(&app, &unlucky, WRONG).await;
    }
    for email in &others {
        post_login(&app, email, WRONG).await;
        post_login(&app, email, WRONG).await;
    }

    assert_eq!(
        post_login(&app, &unlucky, WRONG).await.0,
        StatusCode::TOO_MANY_REQUESTS,
        "the member who spent their own budget is out — privately"
    );
    for email in &others {
        assert_eq!(
            post_login(&app, email, PASSWORD).await.0,
            StatusCode::OK,
            "everyone else at the venue still signs in"
        );
    }
}

/// Breadth, not volume: one source spread thin across many accounts is
/// what the address budget is for, and none of those accounts is
/// anywhere near its own limit.
#[tokio::test]
async fn failing_across_many_accounts_from_one_address_is_rejected() {
    let (state, app) = harness().await;
    let (_id, real, _u) = active_member(&state).await;

    for i in 0..ADDRESS_BREADTH {
        // Identifiers matching no member count too — a source guessing
        // at accounts that do not exist is not a member mistyping.
        let (status, _) = post_login(&app, &format!("no-such-user-{i}@example.com"), WRONG).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "attempt {i} is a plain credential failure, one try per account"
        );
    }

    assert_eq!(
        post_login(&app, "one-more@example.com", WRONG).await.0,
        StatusCode::TOO_MANY_REQUESTS,
        "the address is over its breadth allowance"
    );
    assert_eq!(
        post_login(&app, &real, PASSWORD).await.0,
        StatusCode::TOO_MANY_REQUESTS,
        "and the allowance is the address's, whatever account is named"
    );
}

/// The address signal stays internal: the rejection is byte-identical
/// whether the identifier exists or not, or it becomes the account-
/// existence oracle the requirement forbids.
#[tokio::test]
async fn rejections_do_not_reveal_which_identifiers_exist() {
    let (state, app) = harness().await;
    let (_id, real, _u) = active_member(&state).await;

    for i in 0..ADDRESS_BREADTH {
        post_login(&app, &format!("no-such-user-{i}@example.com"), WRONG).await;
    }

    let (unknown_status, unknown_body) = post_login(&app, "nobody@example.com", WRONG).await;
    let (known_status, known_body) = post_login(&app, &real, WRONG).await;

    assert_eq!(unknown_status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(known_status, unknown_status);
    assert_eq!(known_body, unknown_body);
}

/// Normalization: capitalization is not a fresh budget. Otherwise five
/// failures become five per spelling.
#[tokio::test]
async fn capitalization_does_not_buy_a_fresh_budget() {
    let (state, app) = harness().await;
    let (_id, email, _u) = active_member(&state).await;

    let variants = [
        email.clone(),
        email.to_uppercase(),
        format!(" {email} "),
        email.replace("u-", "U-"),
        email.to_uppercase(),
    ];
    assert_eq!(variants.len(), ACCOUNT_BUDGET);
    for variant in &variants {
        let (status, _) = post_login(&app, variant, WRONG).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    let (status, _) = post_login(&app, &email, WRONG).await;
    assert_eq!(
        status,
        StatusCode::TOO_MANY_REQUESTS,
        "five spellings of one account are one account's budget"
    );
}

// ---------------------------------------------------------------------
// The second factor spends the account's budget
// ---------------------------------------------------------------------

#[tokio::test]
async fn wrong_second_factor_codes_spend_the_accounts_budget() {
    let (state, app) = harness().await;
    let (member_id, email, _u) = active_member(&state).await;

    let pending = state
        .service_context
        .pending_login_service
        .create(member_id, false)
        .await
        .expect("create pending");

    for attempt in 1..=ACCOUNT_BUDGET {
        assert_eq!(
            post_totp(&app, "000000", &pending).await,
            StatusCode::UNAUTHORIZED,
            "wrong code {attempt} is 401"
        );
    }
    assert_eq!(
        post_totp(&app, "000000", &pending).await,
        StatusCode::TOO_MANY_REQUESTS,
        "the 6th wrong code is refused before the code space shrinks any further"
    );

    // Same budget, not a parallel one: the first factor is closed too.
    assert_eq!(
        post_login(&app, &email, PASSWORD).await.0,
        StatusCode::TOO_MANY_REQUESTS,
        "the account budget is shared across surfaces"
    );
}

// Recovery keeping its own budget — a member locked out of login can
// still request a reset — is asserted in tests/recovery_path_test.rs,
// which owns that canon and still passes unchanged against the new key.
