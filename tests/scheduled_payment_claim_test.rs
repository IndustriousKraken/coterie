//! Integration test for the atomic Pending→Processing claim on the
//! scheduled-payment repository (`claim_for_processing`).
//!
//! The claim is a single guarded compare-and-swap
//! (`UPDATE … SET status = 'processing' … WHERE id = ? AND status = 'pending'`):
//! exactly one caller can win it. This test proves the single-flight
//! property — the first claim flips the row and returns `true`, a second
//! claim on the now-`processing` row returns `false` and leaves the
//! status untouched. Mirrors the compare-and-swap idiom of
//! `complete_pending_payment` / `claim_payment_for_refund`.
//!
//! Run: cargo test --test scheduled_payment_claim_test

use std::sync::Arc;

use chrono::{NaiveDate, Utc};
use coterie::{
    domain::{ScheduledPayment, ScheduledPaymentStatus},
    repository::{ScheduledPaymentRepository, SqliteScheduledPaymentRepository},
};
use sqlx::SqlitePool;
use uuid::Uuid;

mod common;
use common::{fresh_pool, make_member};

async fn scheduled_status(pool: &SqlitePool, id: Uuid) -> String {
    sqlx::query_scalar::<_, String>("SELECT status FROM scheduled_payments WHERE id = ?")
        .bind(id.to_string())
        .fetch_one(pool)
        .await
        .expect("query scheduled_payments status")
}

#[tokio::test]
async fn claim_for_processing_is_single_flight() {
    let pool = fresh_pool().await;
    let repo: Arc<dyn ScheduledPaymentRepository> =
        Arc::new(SqliteScheduledPaymentRepository::new(pool.clone()));

    let member_id = make_member(&pool).await;
    let mt_id_str: String = sqlx::query_scalar("SELECT id FROM membership_types LIMIT 1")
        .fetch_one(&pool)
        .await
        .expect("seeded membership_type");
    let mt_id = Uuid::parse_str(&mt_id_str).expect("mt uuid");

    // Insert a Pending scheduled payment.
    let now = Utc::now();
    let sp = ScheduledPayment {
        id: Uuid::new_v4(),
        member_id,
        membership_type_id: mt_id,
        amount_cents: 50_00,
        currency: "USD".to_string(),
        due_date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        status: ScheduledPaymentStatus::Pending,
        retry_count: 0,
        last_attempt_at: None,
        payment_id: None,
        failure_reason: None,
        created_at: now,
        updated_at: now,
    };
    let id = repo.create(sp).await.expect("create scheduled_payment").id;
    assert_eq!(
        scheduled_status(&pool, id).await,
        "pending",
        "row should start Pending"
    );

    // First claim wins: returns true and flips the row to processing.
    let first = repo
        .claim_for_processing(id)
        .await
        .expect("first claim_for_processing");
    assert!(first, "first claim should win (true)");
    assert_eq!(
        scheduled_status(&pool, id).await,
        "processing",
        "winning claim must flip status to processing"
    );

    // Second claim loses: returns false and leaves the row processing.
    let second = repo
        .claim_for_processing(id)
        .await
        .expect("second claim_for_processing");
    assert!(!second, "second claim should lose (false)");
    assert_eq!(
        scheduled_status(&pool, id).await,
        "processing",
        "lost claim must not change status"
    );
}
