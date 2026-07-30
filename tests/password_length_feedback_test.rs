//! Integration tests for the byte-denominated password bound
//! (`password-management`, requirement "Password complexity is validated
//! at change/reset/signup").
//!
//! The incident behind a46 was a report of *no visible warning* when a
//! 200-character password was submitted, while the code reading said a
//! rejection is returned on every set-password path. Inspection cannot
//! settle that disagreement, so each of the four handlers is driven for
//! real here and its response body checked for the message:
//!
//! * `POST /public/signup`            — public signup (JSON)
//! * `POST /portal/profile/password`  — in-portal change (HTMX fragment)
//! * `POST /reset-password`           — reset (full page)
//! * `POST /setup`                    — first-run wizard (JSON)
//!
//! The probe password is deliberately multi-byte: 60 emoji is 243 bytes
//! but only 63 characters, so a message still denominated in characters
//! would be visibly wrong rather than merely imprecise.
//!
//! Run with: cargo test --test password_length_feedback_test

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

/// "Aa1" plus 60 U+1F600 — 63 characters, 243 UTF-8 bytes. Satisfies
/// every complexity rule, so the only thing that can reject it is the
/// length bound.
fn over_long() -> String {
    format!("Aa1{}", "\u{1F600}".repeat(60))
}

/// What every one of the four handlers must say back.
const EXPECTED: &str = "Password must be at most 128 bytes (yours is 243)";

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

/// A router with an admin already observed, so `require_setup` stops
/// redirecting every route to the wizard.
async fn harness() -> (SqlitePool, AppState, Router) {
    let pool = fresh_pool().await;
    let state = build_app_state(pool.clone()).await;
    state
        .admin_exists_observed
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let app = merged_router(state.clone());
    (pool, state, app)
}

async fn active_member(state: &AppState, password: &str) -> Uuid {
    let suffix = Uuid::new_v4();
    let member = state
        .service_context
        .member_repo
        .create(CreateMemberRequest {
            email: format!("u-{suffix}@example.com"),
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
    member.id
}

async fn password_hash(pool: &SqlitePool, member_id: Uuid) -> String {
    sqlx::query_scalar::<_, String>("SELECT password_hash FROM members WHERE id = ?")
        .bind(member_id.to_string())
        .fetch_one(pool)
        .await
        .expect("read password_hash")
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, String) {
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

async fn post_json(app: &Router, uri: &str, body: serde_json::Value) -> (StatusCode, String) {
    send(
        app,
        Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await
}

async fn post_form(
    app: &Router,
    uri: &str,
    session: Option<&str>,
    body: String,
) -> (StatusCode, String) {
    let mut req = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(session) = session {
        req = req.header(header::COOKIE, format!("session={session}"));
    }
    send(app, req.body(Body::from(body)).unwrap()).await
}

/// The whole point of the probe: a message quoting characters would be
/// off by a factor of four here.
fn assert_byte_denominated(surface: &str, body: &str) {
    assert!(
        body.contains(EXPECTED),
        "{surface} must return the byte-denominated rejection; body was: {body}"
    );
    assert!(
        !body.contains("128 characters"),
        "{surface} still describes a byte bound as a character bound"
    );
}

#[tokio::test]
async fn public_signup_reports_the_byte_bound_and_writes_no_member() {
    let (pool, _state, app) = harness().await;

    let (status, body) = post_json(
        &app,
        "/public/signup",
        serde_json::json!({
            "email": "tester@example.com",
            "username": "tester",
            "full_name": "Security Tester",
            "password": over_long(),
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_byte_denominated("public signup", &body);

    let members: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM members")
        .fetch_one(&pool)
        .await
        .expect("count members");
    assert_eq!(
        members, 0,
        "no member — least of all one holding a truncated prefix — may be created"
    );
}

#[tokio::test]
async fn in_portal_change_reports_the_byte_bound_and_leaves_the_hash_untouched() {
    let (pool, state, app) = harness().await;
    let member_id = active_member(&state, PASSWORD).await;
    let before = password_hash(&pool, member_id).await;

    let (session, token) = state
        .service_context
        .auth_service
        .create_session(member_id, 24)
        .await
        .expect("create session");
    let csrf = state
        .service_context
        .csrf_service
        .generate_token(&session.id)
        .await
        .expect("csrf token");
    let session = token;

    let long = over_long();
    let body = format!(
        "csrf_token={}&current_password={}&new_password={}&confirm_password={}",
        urlencode(&csrf),
        urlencode(PASSWORD),
        urlencode(&long),
        urlencode(&long)
    );
    let (_status, body) = post_form(&app, "/portal/profile/password", Some(&session), body).await;
    assert_byte_denominated("in-portal change", &body);

    assert_eq!(
        password_hash(&pool, member_id).await,
        before,
        "an over-length submission must leave the stored credential exactly as it was; \
         storing a hashed prefix would lock the member out of a password they never chose"
    );
}

#[tokio::test]
async fn reset_reports_the_byte_bound_and_does_not_consume_the_token() {
    let (pool, state, app) = harness().await;
    let member_id = active_member(&state, PASSWORD).await;
    let before = password_hash(&pool, member_id).await;
    let token = coterie::auth::email_tokens::create_password_reset_token(
        &pool,
        member_id,
        chrono::Duration::hours(1),
    )
    .await
    .expect("create reset token")
    .token;

    let long = over_long();
    let body = format!(
        "token={}&new_password={}&confirm_password={}",
        urlencode(&token),
        urlencode(&long),
        urlencode(&long)
    );
    let (_status, body) = post_form(&app, "/reset-password", None, body).await;
    assert_byte_denominated("reset", &body);

    assert_eq!(
        password_hash(&pool, member_id).await,
        before,
        "a rejected reset must not touch the stored credential"
    );
    let consumed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM password_reset_tokens WHERE consumed_at IS NOT NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("count consumed tokens");
    assert_eq!(consumed, 0, "the token survives a policy rejection");
}

#[tokio::test]
async fn setup_wizard_reports_the_byte_bound() {
    // No `admin_exists_observed` here: the wizard must still be reachable.
    let pool = fresh_pool().await;
    let state = build_app_state(pool.clone()).await;
    let app = merged_router(state);

    let long = over_long();
    let (status, body) = post_json(
        &app,
        "/setup",
        serde_json::json!({
            "org_name": "Test Org",
            "email": "admin@example.com",
            "username": "admin",
            "full_name": "Admin User",
            "password": long,
            "password_confirm": long,
        }),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_byte_denominated("setup wizard", &body);

    let members: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM members")
        .fetch_one(&pool)
        .await
        .expect("count members");
    assert_eq!(members, 0, "the wizard must not provision on a rejection");
}

/// The ceiling is discoverable before submission, and no field carries a
/// `maxlength`. The second half is the load-bearing one: `maxlength`
/// clips a pasted value with no notice, and on a masked field the user
/// cannot see it happen — they would store a truncated password-manager
/// paste and be locked out of a credential they never chose. That is the
/// silent truncation the server-side rule forbids, reintroduced at the
/// client. Asserted over the template sources so a future edit adding
/// the attribute back fails here rather than in production.
#[test]
fn set_password_forms_state_the_ceiling_and_carry_no_maxlength() {
    let surfaces = [
        ("setup wizard", include_str!("../templates/auth/setup.html")),
        (
            "reset",
            include_str!("../templates/auth/reset_password.html"),
        ),
        (
            "in-portal change",
            include_str!("../templates/portal/profile.html"),
        ),
    ];

    for (surface, source) in surfaces {
        assert!(
            source.contains("10-128 bytes"),
            "{surface} must state the ceiling before the form is submitted"
        );
        assert!(
            !source.contains("maxlength="),
            "{surface} must not carry a maxlength attribute"
        );
    }
}
