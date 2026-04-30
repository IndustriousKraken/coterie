//! Integration tests for the two repo methods that power the admin
//! billing dashboard:
//!
//!   - `PaymentRepository::revenue_by_month` — sums Completed payment
//!     cents grouped by (year, month, payment_type) over the last N
//!     months. Refunded / Pending / Failed are excluded.
//!   - `ScheduledPaymentRepository::list_failures_since` — Failed
//!     scheduled payments whose last attempt landed in [since, now].
//!
//! Run: cargo test --test billing_dashboard_test

use std::sync::Arc;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use coterie::{
    domain::{
        CreateMemberRequest, MembershipType, Payer, Payment, PaymentKind, PaymentMethod,
        PaymentStatus, ScheduledPayment, ScheduledPaymentStatus, StripeRef,
    },
    repository::{
        MemberRepository, PaymentRepository, ScheduledPaymentRepository,
        SqliteMemberRepository, SqlitePaymentRepository,
        SqliteScheduledPaymentRepository,
    },
};
use sqlx::{Executor, SqlitePool};
use uuid::Uuid;

async fn fresh_pool() -> SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .after_connect(|conn, _| {
            Box::pin(async move {
                conn.execute("PRAGMA foreign_keys = ON").await?;
                Ok(())
            })
        })
        .connect("sqlite::memory:")
        .await
        .expect(":memory:");
    sqlx::migrate!("./migrations").run(&pool).await.expect("migrate");
    pool
}

async fn make_member(pool: &SqlitePool) -> Uuid {
    let repo = SqliteMemberRepository::new(pool.clone());
    let m = repo
        .create(CreateMemberRequest {
            email: format!("u-{}@example.com", Uuid::new_v4()),
            username: format!("u_{}", Uuid::new_v4().simple()),
            full_name: "Test".to_string(),
            password: "p4ssword_long_enough".to_string(),
            membership_type: MembershipType::Regular,
        })
        .await
        .unwrap();
    m.id
}

/// Insert a Payment and stamp `paid_at` directly via SQL — the
/// repository's create() doesn't accept paid_at-in-the-past for
/// historical bucket testing, but we need it for revenue_by_month.
async fn insert_completed_payment(
    pool: &SqlitePool,
    repo: &Arc<dyn PaymentRepository>,
    member_id: Uuid,
    amount_cents: i64,
    kind: PaymentKind,
    paid_at: DateTime<Utc>,
) -> Uuid {
    let id = Uuid::new_v4();
    let payment = Payment {
        id,
        payer: Payer::Member(member_id),
        amount_cents,
        currency: "USD".to_string(),
        status: PaymentStatus::Completed,
        payment_method: PaymentMethod::Stripe,
        external_id: Some(StripeRef::PaymentIntent(format!("pi_test_{}", id.simple()))),
        description: "test".to_string(),
        kind,
        paid_at: Some(paid_at),
        created_at: paid_at,
        updated_at: paid_at,
    };
    repo.create(payment).await.unwrap();
    // Override paid_at to land in the historical bucket we want.
    sqlx::query("UPDATE payments SET paid_at = ?, status = 'Completed' WHERE id = ?")
        .bind(paid_at.naive_utc())
        .bind(id.to_string())
        .execute(pool).await.unwrap();
    id
}

// --------------------------------------------------------------------
// revenue_by_month
// --------------------------------------------------------------------

#[tokio::test]
async fn revenue_by_month_groups_dues_and_donations_separately() {
    let pool = fresh_pool().await;
    let repo: Arc<dyn PaymentRepository> = Arc::new(SqlitePaymentRepository::new(pool.clone()));
    let m = make_member(&pool).await;

    let last_month = Utc::now() - Duration::days(20);
    insert_completed_payment(&pool, &repo, m, 50_00, PaymentKind::Membership, last_month).await;
    insert_completed_payment(&pool, &repo, m, 25_00, PaymentKind::Membership, last_month).await;
    insert_completed_payment(&pool, &repo, m, 100_00, PaymentKind::Donation { campaign_id: None }, last_month).await;

    let buckets = repo.revenue_by_month(12).await.unwrap();
    assert_eq!(buckets.len(), 2, "should have two buckets: Membership + Donation");

    let dues = buckets.iter().find(|b| b.payment_type == "membership").unwrap();
    let donations = buckets.iter().find(|b| b.payment_type == "donation").unwrap();

    assert_eq!(dues.total_cents, 75_00);
    assert_eq!(dues.payment_count, 2);
    assert_eq!(donations.total_cents, 100_00);
    assert_eq!(donations.payment_count, 1);
}

#[tokio::test]
async fn revenue_by_month_excludes_refunded_and_pending() {
    let pool = fresh_pool().await;
    let repo: Arc<dyn PaymentRepository> = Arc::new(SqlitePaymentRepository::new(pool.clone()));
    let m = make_member(&pool).await;
    let recent = Utc::now() - Duration::days(5);

    let completed = insert_completed_payment(&pool, &repo, m, 50_00, PaymentKind::Membership, recent).await;
    let to_refund = insert_completed_payment(&pool, &repo, m, 75_00, PaymentKind::Membership, recent).await;
    let to_pend = insert_completed_payment(&pool, &repo, m, 99_00, PaymentKind::Membership, recent).await;

    // Flip the second one to Refunded and the third one to Pending.
    sqlx::query("UPDATE payments SET status = 'Refunded' WHERE id = ?")
        .bind(to_refund.to_string()).execute(&pool).await.unwrap();
    sqlx::query("UPDATE payments SET status = 'Pending', paid_at = NULL WHERE id = ?")
        .bind(to_pend.to_string()).execute(&pool).await.unwrap();

    let buckets = repo.revenue_by_month(12).await.unwrap();
    let dues = buckets.iter()
        .find(|b| b.payment_type == "membership")
        .expect("dues bucket present");

    // Only the Completed row counts.
    assert_eq!(dues.payment_count, 1);
    assert_eq!(dues.total_cents, 50_00);

    // Sanity: the Completed row matches the one we kept.
    assert!(repo.find_by_id(completed).await.unwrap().is_some());
}

#[tokio::test]
async fn revenue_by_month_orders_newest_first() {
    let pool = fresh_pool().await;
    let repo: Arc<dyn PaymentRepository> = Arc::new(SqlitePaymentRepository::new(pool.clone()));
    let m = make_member(&pool).await;

    let three_months_ago = Utc::now() - Duration::days(90);
    let one_month_ago = Utc::now() - Duration::days(20);
    insert_completed_payment(&pool, &repo, m, 10_00, PaymentKind::Membership, three_months_ago).await;
    insert_completed_payment(&pool, &repo, m, 20_00, PaymentKind::Membership, one_month_ago).await;

    let buckets = repo.revenue_by_month(12).await.unwrap();
    // Two distinct months, two buckets — newer first.
    assert_eq!(buckets.len(), 2);
    let first = &buckets[0];
    let second = &buckets[1];
    assert!(
        (first.year, first.month) > (second.year, second.month),
        "newer month must come first: got {:?}/{:?} then {:?}/{:?}",
        first.year, first.month, second.year, second.month,
    );
}

#[tokio::test]
async fn revenue_by_month_horizon_drops_old_payments() {
    let pool = fresh_pool().await;
    let repo: Arc<dyn PaymentRepository> = Arc::new(SqlitePaymentRepository::new(pool.clone()));
    let m = make_member(&pool).await;

    // 14 months ago — outside a 12-month horizon.
    let way_back = Utc::now() - Duration::days(14 * 30);
    insert_completed_payment(&pool, &repo, m, 999_00, PaymentKind::Membership, way_back).await;

    // Within the horizon.
    let recent = Utc::now() - Duration::days(10);
    insert_completed_payment(&pool, &repo, m, 50_00, PaymentKind::Membership, recent).await;

    let buckets = repo.revenue_by_month(12).await.unwrap();
    let total_cents: i64 = buckets.iter().map(|b| b.total_cents).sum();
    assert_eq!(total_cents, 50_00, "old payment must be excluded by horizon");
}

// --------------------------------------------------------------------
// ScheduledPayment::list_failures_since
// --------------------------------------------------------------------

#[tokio::test]
async fn list_failures_since_returns_only_failed_in_window() {
    let pool = fresh_pool().await;
    let sched_repo: Arc<dyn ScheduledPaymentRepository> =
        Arc::new(SqliteScheduledPaymentRepository::new(pool.clone()));

    let member_id = make_member(&pool).await;
    // Use the seeded membership_type_id from migration 001.
    let mt_id_str: String = sqlx::query_scalar(
        "SELECT id FROM membership_types LIMIT 1"
    ).fetch_one(&pool).await.unwrap();
    let mt_id = Uuid::parse_str(&mt_id_str).unwrap();

    // Helper to create + transition a scheduled payment.
    async fn insert_scheduled(
        repo: &Arc<dyn ScheduledPaymentRepository>,
        pool: &SqlitePool,
        member_id: Uuid,
        mt_id: Uuid,
        status: ScheduledPaymentStatus,
        last_attempt_at: Option<DateTime<Utc>>,
    ) -> Uuid {
        let due_date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let now = Utc::now();
        let sp = ScheduledPayment {
            id: Uuid::new_v4(),
            member_id,
            membership_type_id: mt_id,
            amount_cents: 50_00,
            currency: "USD".to_string(),
            due_date,
            status: ScheduledPaymentStatus::Pending,
            retry_count: 0,
            last_attempt_at: None,
            payment_id: None,
            failure_reason: None,
            created_at: now,
            updated_at: now,
        };
        let row = repo.create(sp).await.unwrap();
        // Push to the desired status + stamp last_attempt_at.
        if status != ScheduledPaymentStatus::Pending {
            sqlx::query(
                "UPDATE scheduled_payments \
                 SET status = ?, last_attempt_at = ?, retry_count = 1, \
                     failure_reason = 'card_declined' \
                 WHERE id = ?",
            )
            .bind(status.as_str())
            .bind(last_attempt_at.map(|d| d.naive_utc()))
            .bind(row.id.to_string())
            .execute(pool).await.unwrap();
        }
        row.id
    }

    let recent = Utc::now() - Duration::days(5);
    let stale = Utc::now() - Duration::days(120);

    let recent_fail = insert_scheduled(
        &sched_repo, &pool, member_id, mt_id,
        ScheduledPaymentStatus::Failed, Some(recent),
    ).await;
    let _stale_fail = insert_scheduled(
        &sched_repo, &pool, member_id, mt_id,
        ScheduledPaymentStatus::Failed, Some(stale),
    ).await;
    let _completed = insert_scheduled(
        &sched_repo, &pool, member_id, mt_id,
        ScheduledPaymentStatus::Completed, Some(recent),
    ).await;
    let _pending = insert_scheduled(
        &sched_repo, &pool, member_id, mt_id,
        ScheduledPaymentStatus::Pending, None,
    ).await;

    let since = Utc::now() - Duration::days(90);
    let failures = sched_repo.list_failures_since(since).await.unwrap();

    // Only the recent failure should land in the result.
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].id, recent_fail);
    assert_eq!(failures[0].status, ScheduledPaymentStatus::Failed);
    assert_eq!(failures[0].retry_count, 1);
    assert_eq!(failures[0].failure_reason.as_deref(), Some("card_declined"));
}

#[tokio::test]
async fn list_failures_since_orders_newest_first() {
    let pool = fresh_pool().await;
    let sched_repo: Arc<dyn ScheduledPaymentRepository> =
        Arc::new(SqliteScheduledPaymentRepository::new(pool.clone()));
    let member_id = make_member(&pool).await;
    let mt_id_str: String = sqlx::query_scalar("SELECT id FROM membership_types LIMIT 1")
        .fetch_one(&pool).await.unwrap();
    let mt_id = Uuid::parse_str(&mt_id_str).unwrap();

    // Insert two failed rows with different last_attempt_at.
    for days_ago in &[30i64, 5, 60] {
        let ts = Utc::now() - Duration::days(*days_ago);
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
            created_at: ts,
            updated_at: ts,
        };
        let row = sched_repo.create(sp).await.unwrap();
        sqlx::query(
            "UPDATE scheduled_payments SET status = 'failed', last_attempt_at = ? WHERE id = ?",
        )
        .bind(ts.naive_utc())
        .bind(row.id.to_string())
        .execute(&pool).await.unwrap();
    }

    let since = Utc::now() - Duration::days(90);
    let failures = sched_repo.list_failures_since(since).await.unwrap();
    assert_eq!(failures.len(), 3);
    // Newest first — the 5-day-old failure leads.
    let mut last = failures[0].last_attempt_at.unwrap();
    for f in &failures[1..] {
        let cur = f.last_attempt_at.unwrap();
        assert!(cur <= last, "expected DESC order");
        last = cur;
    }
}
