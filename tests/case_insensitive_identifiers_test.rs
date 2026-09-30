//! `member-identifiers`: email and username match without regard to case,
//! are unique the same way, and are stored exactly as entered.
//!
//! Motivated by two real lockouts: `Mia@…` typing `mia@…`, and `RelicIX`
//! typing a different case. Both accounts were valid; every attempt was
//! denied as an unknown identifier, so no password was ever checked.
//!
//! Run with: cargo test --test case_insensitive_identifiers_test

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
    Router,
};
use coterie::{
    api::state::AppState,
    domain::{CreateMemberRequest, MemberStatus, UpdateMemberRequest},
    error::AppError,
    repository::{MemberRepository, SqliteMemberRepository},
    service::member_service::ImportRow,
};
use sqlx::SqlitePool;
use tower::ServiceExt;
use uuid::Uuid;

mod common;
use common::{build_app_state, fresh_pool, merged_router};

const PASSWORD: &str = "Corr3ctHorseBattery";
const EMAIL: &str = "Mia@RupeCreativeHouse.com";
const USERNAME: &str = "RelicIX";

async fn harness() -> (SqlitePool, AppState, Router) {
    let pool = fresh_pool().await;
    let state = build_app_state(pool.clone()).await;
    state
        .admin_exists_observed
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let app = merged_router(state.clone());
    (pool, state, app)
}

async fn create(state: &AppState, email: &str, username: &str) -> Result<Uuid, AppError> {
    let member = state
        .service_context
        .member_repo
        .create(CreateMemberRequest {
            email: email.to_string(),
            username: username.to_string(),
            full_name: "Mixed Case".to_string(),
            password: PASSWORD.to_string(),
            membership_type_id: None,
            ..Default::default()
        })
        .await?;
    Ok(member.id)
}

async fn active_member(state: &AppState) -> Uuid {
    let id = create(state, EMAIL, USERNAME).await.expect("create member");
    state
        .service_context
        .member_repo
        .update(
            id,
            UpdateMemberRequest {
                status: Some(MemberStatus::Active),
                ..Default::default()
            },
        )
        .await
        .expect("activate member");
    id
}

async fn post_json(app: &Router, uri: &str, body: serde_json::Value) -> StatusCode {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

async fn post_form(app: &Router, uri: &str, body: String) -> StatusCode {
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
    let _ = to_bytes(resp.into_body(), usize::MAX).await;
    status
}

fn is_unique_violation(e: &AppError) -> bool {
    matches!(e, AppError::Database(sqlx::Error::Database(db)) if db.is_unique_violation())
}

// ---------------------------------------------------------------------
// Matching
// ---------------------------------------------------------------------

#[tokio::test]
async fn repository_lookups_ignore_case() {
    let (pool, state, _) = harness().await;
    let id = active_member(&state).await;
    let repo = SqliteMemberRepository::new(pool.clone());

    for probe in [
        "mia@rupecreativehouse.com",
        "MIA@RUPECREATIVEHOUSE.COM",
        EMAIL,
    ] {
        let found = repo.find_by_email(probe).await.unwrap();
        assert_eq!(found.map(|m| m.id), Some(id), "find_by_email({probe})");
    }
    for probe in ["relicix", "RELICIX", "RelicIx", USERNAME] {
        let found = repo.find_by_username(probe).await.unwrap();
        assert_eq!(found.map(|m| m.id), Some(id), "find_by_username({probe})");
    }
    // Not a prefix or fuzzy match — only case is ignored.
    assert!(repo.find_by_username("Relic").await.unwrap().is_none());

    // The password-hash lookup the API login and signup use directly.
    let hash = coterie::auth::get_password_hash(&pool, "mia@rupecreativehouse.com")
        .await
        .unwrap();
    assert_eq!(hash.map(|(mid, _)| mid), Some(id));
}

#[tokio::test]
async fn web_login_succeeds_with_a_differently_cased_email_or_username() {
    let (_, state, app) = harness().await;
    active_member(&state).await;

    for identifier in ["mia@rupecreativehouse.com", "relicix"] {
        let status = post_json(
            &app,
            "/login",
            serde_json::json!({ "username": identifier, "password": PASSWORD }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "web login as {identifier}");
    }
}

#[tokio::test]
async fn api_login_succeeds_with_a_differently_cased_email() {
    let (_, state, app) = harness().await;
    active_member(&state).await;

    let status = post_json(
        &app,
        "/auth/login",
        serde_json::json!({ "email": "mia@rupecreativehouse.com", "password": PASSWORD }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_wrong_password_is_still_denied_for_a_case_variant() {
    // Case-insensitivity loosens the account lookup, never the password.
    let (_, state, app) = harness().await;
    active_member(&state).await;

    let status = post_json(
        &app,
        "/login",
        serde_json::json!({ "username": "relicix", "password": "Wr0ngPassword!!" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_reset_requested_in_a_different_case_is_issued() {
    let (pool, state, app) = harness().await;
    let id = active_member(&state).await;

    post_form(
        &app,
        "/forgot-password",
        "email=mia%40rupecreativehouse.com".to_string(),
    )
    .await;

    let tokens: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM password_reset_tokens WHERE member_id = ?")
            .bind(id.to_string())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        tokens, 1,
        "the reset must reach the member, not silently vanish"
    );
}

// ---------------------------------------------------------------------
// Uniqueness
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_case_variant_username_or_email_is_refused_as_a_duplicate() {
    let (_, state, _) = harness().await;
    active_member(&state).await;

    let err = create(&state, "someone.else@example.com", "relicix")
        .await
        .expect_err("case-variant username must be refused");
    assert!(is_unique_violation(&err), "username: {err:?}");

    let err = create(&state, "MIA@rupecreativehouse.com", "someone_else")
        .await
        .expect_err("case-variant email must be refused");
    assert!(is_unique_violation(&err), "email: {err:?}");
}

#[tokio::test]
async fn signup_refuses_a_case_variant_email_with_the_generic_conflict() {
    let (_, state, app) = harness().await;
    active_member(&state).await;

    let status = post_json(
        &app,
        "/public/signup",
        serde_json::json!({
            "email": "mia@rupecreativehouse.com",
            "username": "brand_new_name",
            "full_name": "Someone",
            "password": PASSWORD,
            "membership_type_slug": "member",
        }),
    )
    .await;
    // The same generic 409 an exact duplicate gets — not a second account,
    // and not a 500 from an unrecognised constraint.
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn bulk_import_reports_a_case_variant_row_as_existing() {
    let (_, state, _) = harness().await;
    let admin = active_member(&state).await;

    let row = ImportRow {
        email: "mia@rupecreativehouse.com".to_string(),
        username: "unrelated_name".to_string(),
        full_name: "Import".to_string(),
        membership_type_slug: "member".to_string(),
        status: None,
        notes: None,
        discord_id: None,
        dues_paid_until: None,
        stripe_customer_id: None,
        stripe_subscription_id: None,
        joined_at: None,
        email_verified_at: None,
        parse_error: None,
    };
    let summary = state
        .service_context
        .member_service
        .bulk_import(admin, "members.csv", vec![row])
        .await
        .unwrap();
    assert_eq!(summary.succeeded, 0);
    assert_eq!(summary.failed, 1);
    assert_eq!(summary.failures[0].reason, "Email already exists");
}

#[tokio::test]
async fn the_migration_refuses_existing_case_only_duplicates() {
    // Build the schema up to (but not including) 049, insert two members
    // differing only by case, then apply 049: it must fail, not guess.
    // One connection: each pooled connection to `:memory:` is its own
    // empty database, so a second connection would not see the schema.
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let migrator = sqlx::migrate!("./migrations");
    let target = migrator
        .iter()
        .find(|m| {
            m.description
                .contains("case insensitive member identifiers")
        })
        .expect("migration 049 present")
        .version;

    let mut before = sqlx::migrate!("./migrations");
    before.migrations = std::borrow::Cow::Owned(
        migrator
            .iter()
            .filter(|m| m.version < target)
            .cloned()
            .collect(),
    );
    before.run(&pool).await.unwrap();

    let repo = SqliteMemberRepository::new(pool.clone());
    for (email, username) in [("dup@example.com", "one"), ("DUP@example.com", "two")] {
        repo.create(CreateMemberRequest {
            email: email.to_string(),
            username: username.to_string(),
            full_name: "Dup".to_string(),
            password: PASSWORD.to_string(),
            membership_type_id: None,
            ..Default::default()
        })
        .await
        .unwrap();
    }

    assert!(
        migrator.run(&pool).await.is_err(),
        "049 must stop rather than proceed over case-only duplicates"
    );
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM members")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 2, "no row may be dropped or altered");
}

// ---------------------------------------------------------------------
// Preservation
// ---------------------------------------------------------------------

#[tokio::test]
async fn identifiers_keep_the_case_they_were_entered_with() {
    let (pool, state, app) = harness().await;
    let id = active_member(&state).await;

    post_json(
        &app,
        "/login",
        serde_json::json!({ "username": "relicix", "password": PASSWORD }),
    )
    .await;
    let _ = create(&state, "mia@rupecreativehouse.com", "relicix").await;

    let (email, username): (String, String) =
        sqlx::query_as("SELECT email, username FROM members WHERE id = ?")
            .bind(id.to_string())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(email, EMAIL);
    assert_eq!(username, USERNAME);
}
