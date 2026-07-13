//! Integration tests for the input-bounding invariant on
//! `POST /public/signup` (see the `public-signup` spec, requirement
//! "Signup bounds and validates its input fields").
//!
//! `/public/signup` is unauthenticated and previously bounded only the
//! email's `@` shape. These tests assert oversized/empty `email`,
//! `username`, and `full_name` are now rejected with `400` before any
//! member row is written, and that a normal-length valid signup still
//! succeeds (the bounds are not too tight).

use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use sqlx::SqlitePool;
use tower::ServiceExt;

mod common;
use common::{build_app_state, fresh_pool};

/// POST a JSON body to `/public/signup` and return the status. The app
/// is built without the top-level CSRF layer (that lives in `main.rs`),
/// and `bot_challenge` defaults to `DisabledVerifier`, so the request
/// reaches the handler with no token.
async fn post_signup(app: Router, body: serde_json::Value) -> StatusCode {
    let req = Request::builder()
        .method("POST")
        .uri("/public/signup")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    app.oneshot(req).await.unwrap().status()
}

async fn member_count(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM members")
        .fetch_one(pool)
        .await
        .expect("count members")
}

/// Insert a membership type with the given slug and `is_active` flag so
/// signup slug-resolution tests can exercise active vs. inactive types.
async fn insert_membership_type(pool: &SqlitePool, slug: &str, is_active: bool) {
    sqlx::query(
        "INSERT INTO membership_types \
           (id, name, slug, description, sort_order, is_active, fee_cents, billing_period, \
            created_at, updated_at) \
         VALUES (?, ?, ?, 'test tier', 50, ?, 0, 'monthly', datetime('now'), datetime('now'))",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(slug)
    .bind(slug)
    .bind(is_active)
    .execute(pool)
    .await
    .expect("insert membership type");
}

fn valid_body() -> serde_json::Value {
    serde_json::json!({
        "email": "newbie@example.com",
        "username": "newbie",
        "full_name": "New Bie",
        "password": "Sup3rSecret!",
    })
}

#[tokio::test]
async fn oversized_email_is_rejected() {
    let pool = fresh_pool().await;
    let app = coterie::api::create_app(build_app_state(pool.clone()).await);

    let mut body = valid_body();
    // 255 chars before the domain — total well over the 254 cap.
    body["email"] = serde_json::json!(format!("{}@example.com", "a".repeat(255)));

    let status = post_signup(app, body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "email > 254 must 400");
    assert_eq!(member_count(&pool).await, 0, "no member should be created");
}

#[tokio::test]
async fn oversized_full_name_is_rejected() {
    let pool = fresh_pool().await;
    let app = coterie::api::create_app(build_app_state(pool.clone()).await);

    let mut body = valid_body();
    body["full_name"] = serde_json::json!("n".repeat(201));

    let status = post_signup(app, body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "full_name > 200 must 400");
    assert_eq!(member_count(&pool).await, 0, "no member should be created");
}

#[tokio::test]
async fn oversized_username_is_rejected() {
    let pool = fresh_pool().await;
    let app = coterie::api::create_app(build_app_state(pool.clone()).await);

    let mut body = valid_body();
    body["username"] = serde_json::json!("u".repeat(101));

    let status = post_signup(app, body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "username > 100 must 400");
    assert_eq!(member_count(&pool).await, 0, "no member should be created");
}

#[tokio::test]
async fn empty_username_is_rejected() {
    let pool = fresh_pool().await;
    let app = coterie::api::create_app(build_app_state(pool.clone()).await);

    let mut body = valid_body();
    body["username"] = serde_json::json!("   "); // whitespace-only

    let status = post_signup(app, body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "blank username must 400");
    assert_eq!(member_count(&pool).await, 0, "no member should be created");
}

#[tokio::test]
async fn empty_full_name_is_rejected() {
    let pool = fresh_pool().await;
    let app = coterie::api::create_app(build_app_state(pool.clone()).await);

    let mut body = valid_body();
    body["full_name"] = serde_json::json!(""); // empty

    let status = post_signup(app, body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "blank full_name must 400");
    assert_eq!(member_count(&pool).await, 0, "no member should be created");
}

#[tokio::test]
async fn valid_normal_length_signup_succeeds() {
    let pool = fresh_pool().await;
    let app = coterie::api::create_app(build_app_state(pool.clone()).await);

    let status = post_signup(app, valid_body()).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "a normal-length valid signup must still create a member"
    );
    assert_eq!(member_count(&pool).await, 1, "one member should be created");
}

#[tokio::test]
async fn inactive_membership_type_slug_is_rejected() {
    let pool = fresh_pool().await;
    insert_membership_type(&pool, "retired-tier", false).await;
    let app = coterie::api::create_app(build_app_state(pool.clone()).await);

    let mut body = valid_body();
    body["membership_type_slug"] = serde_json::json!("retired-tier");

    let status = post_signup(app, body).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "an inactive membership type slug must 400"
    );
    assert_eq!(member_count(&pool).await, 0, "no member should be created");
}

#[tokio::test]
async fn unknown_membership_type_slug_is_rejected() {
    let pool = fresh_pool().await;
    let app = coterie::api::create_app(build_app_state(pool.clone()).await);

    let mut body = valid_body();
    body["membership_type_slug"] = serde_json::json!("no-such-tier");

    let status = post_signup(app, body).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "an unknown membership type slug must 400"
    );
    assert_eq!(member_count(&pool).await, 0, "no member should be created");
}

#[tokio::test]
async fn active_membership_type_slug_succeeds_as_pending() {
    let pool = fresh_pool().await;
    insert_membership_type(&pool, "active-tier", true).await;
    let app = coterie::api::create_app(build_app_state(pool.clone()).await);

    let mut body = valid_body();
    body["membership_type_slug"] = serde_json::json!("active-tier");

    let status = post_signup(app, body).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "an active membership type slug must still succeed"
    );
    assert_eq!(member_count(&pool).await, 1, "one member should be created");

    let member_status: String = sqlx::query_scalar("SELECT status FROM members LIMIT 1")
        .fetch_one(&pool)
        .await
        .expect("member status");
    assert_eq!(member_status, "Pending", "new signup must be Pending");
}

#[tokio::test]
async fn omitted_slug_takes_org_default() {
    let pool = fresh_pool().await;
    let app = coterie::api::create_app(build_app_state(pool.clone()).await);

    // valid_body() omits membership_type_slug entirely.
    let status = post_signup(app, valid_body()).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "an omitted slug must fall back to the org default and succeed"
    );

    // The member lands on the first active type by sort_order — the
    // seeded `member` type (sort_order 0).
    let slug: String = sqlx::query_scalar(
        "SELECT mt.slug FROM members m \
         JOIN membership_types mt ON mt.id = m.membership_type_id LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("resolved membership type slug");
    assert_eq!(slug, "member", "omitted slug takes the first active type");
}
