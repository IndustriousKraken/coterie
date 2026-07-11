//! Integration tests for `GET /public/membership-types` (see the
//! `public-membership-types` spec). The endpoint feeds the marketing
//! site's join form: active types only, sort_order-ordered, public
//! fields only (slug is the public identifier — no internal ids).

use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use sqlx::SqlitePool;
use tower::ServiceExt;

mod common;
use common::{build_app_state, fresh_pool};

async fn get_membership_types(app: Router) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method("GET")
        .uri("/public/membership-types")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

/// Clear whatever the migrations seeded so each test controls the table.
async fn clear_types(pool: &SqlitePool) {
    sqlx::query("DELETE FROM membership_types")
        .execute(pool)
        .await
        .expect("clear membership_types");
}

async fn insert_type(
    pool: &SqlitePool,
    slug: &str,
    name: &str,
    sort_order: i32,
    is_active: bool,
    fee_cents: i32,
) {
    sqlx::query(
        "INSERT INTO membership_types \
           (id, name, slug, description, sort_order, is_active, fee_cents, billing_period, \
            created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, 'monthly', datetime('now'), datetime('now'))",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(name)
    .bind(slug)
    .bind(format!("{name} tier"))
    .bind(sort_order)
    .bind(is_active)
    .bind(fee_cents)
    .execute(pool)
    .await
    .expect("insert membership type");
}

#[tokio::test]
async fn active_types_listed_in_sort_order_with_public_fields_only() {
    let pool = fresh_pool().await;
    clear_types(&pool).await;
    // Insert out of order to prove ordering comes from sort_order.
    insert_type(&pool, "patron", "Patron", 2, true, 8000).await;
    insert_type(&pool, "member", "Member", 1, true, 4500).await;
    insert_type(&pool, "legacy", "Legacy", 0, false, 100).await;

    let app = coterie::api::create_app(build_app_state(pool.clone()).await);
    let (status, json) = get_membership_types(app).await;

    assert_eq!(status, StatusCode::OK);
    let types = json.as_array().expect("array response");
    assert_eq!(types.len(), 2, "inactive type must be excluded");
    assert_eq!(types[0]["slug"], "member", "sort_order 1 first");
    assert_eq!(types[1]["slug"], "patron", "sort_order 2 second");

    let first = types[0].as_object().unwrap();
    for field in ["slug", "name", "description", "fee_cents", "currency", "billing_period"] {
        assert!(first.contains_key(field), "missing public field {field}");
    }
    assert_eq!(types[0]["fee_cents"], 4500);
    assert_eq!(types[0]["currency"], "USD");
    assert_eq!(types[0]["billing_period"], "monthly");
    assert!(
        !first.contains_key("id") && !first.contains_key("is_active"),
        "internal fields must not be exposed"
    );
}

#[tokio::test]
async fn empty_table_returns_empty_array() {
    let pool = fresh_pool().await;
    clear_types(&pool).await;

    let app = coterie::api::create_app(build_app_state(pool.clone()).await);
    let (status, json) = get_membership_types(app).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json, serde_json::json!([]), "empty list, not an error");
}
