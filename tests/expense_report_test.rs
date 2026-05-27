//! Report-shape tests: monthly + annual aggregation queries against
//! the expense + payment data fed in by a fixture dataset.

use std::sync::Arc;

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use coterie::{
    domain::{
        CreateExpenseAccountRequest, CreateExpenseCategoryRequest, CreateExpenseRequest, Payer,
        Payment, PaymentKind, PaymentMethod, PaymentStatus, StripeRef,
    },
    repository::{
        expense_repository::DateRange, ExpenseAccountRepository, ExpenseCategoryRepository,
        ExpenseRepository, PaymentRepository, SqliteExpenseAccountRepository,
        SqliteExpenseCategoryRepository, SqliteExpenseRepository, SqlitePaymentRepository,
    },
};
use sqlx::SqlitePool;
use uuid::Uuid;

mod common;
use common::{fresh_pool, make_member};

async fn insert_completed_payment(
    pool: &SqlitePool,
    repo: &Arc<dyn PaymentRepository>,
    member: Uuid,
    amount_cents: i64,
    kind: PaymentKind,
    paid_at: DateTime<Utc>,
) -> Uuid {
    let id = Uuid::new_v4();
    let payment = Payment {
        id,
        payer: Payer::Member(member),
        amount_cents,
        currency: "USD".into(),
        status: PaymentStatus::Completed,
        payment_method: PaymentMethod::Stripe,
        external_id: Some(StripeRef::PaymentIntent(format!("pi_{}", id.simple()))),
        description: "test".into(),
        kind,
        paid_at: Some(paid_at),
        created_at: paid_at,
        updated_at: paid_at,
    };
    repo.create(payment).await.unwrap();
    sqlx::query("UPDATE payments SET paid_at = ?, status = 'Completed' WHERE id = ?")
        .bind(paid_at.naive_utc())
        .bind(id.to_string())
        .execute(pool)
        .await
        .unwrap();
    id
}

async fn month_range(year: i32, month: u32) -> DateRange {
    let start = Utc
        .from_local_datetime(
            &NaiveDate::from_ymd_opt(year, month, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
        )
        .unwrap();
    let (ny, nm) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let end = Utc
        .from_local_datetime(
            &NaiveDate::from_ymd_opt(ny, nm, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
        )
        .unwrap();
    DateRange { start, end }
}

async fn year_range(year: i32) -> DateRange {
    let start = Utc
        .from_local_datetime(
            &NaiveDate::from_ymd_opt(year, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
        )
        .unwrap();
    let end = Utc
        .from_local_datetime(
            &NaiveDate::from_ymd_opt(year + 1, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
        )
        .unwrap();
    DateRange { start, end }
}

#[tokio::test]
async fn monthly_report_sums_correctly() {
    // Mirrors the spec's worked example:
    // GIVEN three expenses ($30 + $50 + $20) on the same account,
    // two on Supplies and one on Software, plus a $200 completed
    // payment in the month.
    let pool = fresh_pool().await;
    let cat_repo: Arc<dyn ExpenseCategoryRepository> =
        Arc::new(SqliteExpenseCategoryRepository::new(pool.clone()));
    let acc_repo: Arc<dyn ExpenseAccountRepository> =
        Arc::new(SqliteExpenseAccountRepository::new(pool.clone()));
    let exp_repo: Arc<dyn ExpenseRepository> = Arc::new(SqliteExpenseRepository::new(pool.clone()));
    let pay_repo: Arc<dyn PaymentRepository> = Arc::new(SqlitePaymentRepository::new(pool.clone()));

    let actor = make_member(&pool).await;
    let supplies = cat_repo
        .create(CreateExpenseCategoryRequest {
            name: "Supplies".into(),
            slug: None,
        })
        .await
        .unwrap();
    let software = cat_repo
        .create(CreateExpenseCategoryRequest {
            name: "Software".into(),
            slug: None,
        })
        .await
        .unwrap();
    let card1 = acc_repo
        .create(CreateExpenseAccountRequest {
            name: "Card 1".into(),
        })
        .await
        .unwrap();

    let day = Utc.with_ymd_and_hms(2026, 4, 10, 12, 0, 0).unwrap();
    for (amt, cat) in [
        (3_000i64, supplies.id),
        (5_000, supplies.id),
        (2_000, software.id),
    ] {
        exp_repo
            .create(
                actor,
                CreateExpenseRequest {
                    spent_at: day,
                    amount_cents: amt,
                    currency: None,
                    description: "x".into(),
                    category_id: cat,
                    account_id: card1.id,
                    notes: None,
                },
            )
            .await
            .unwrap();
    }

    insert_completed_payment(
        &pool,
        &pay_repo,
        actor,
        20_000,
        PaymentKind::Membership,
        day,
    )
    .await;

    let range = month_range(2026, 4).await;

    let by_acct = exp_repo.sum_by_account(range).await.unwrap();
    assert_eq!(by_acct.len(), 1);
    assert_eq!(by_acct[0].total_cents, 10_000);

    let by_cat = exp_repo.sum_by_category(range).await.unwrap();
    let sup = by_cat.iter().find(|s| s.key_id == supplies.id).unwrap();
    let sw = by_cat.iter().find(|s| s.key_id == software.id).unwrap();
    assert_eq!(sup.total_cents, 8_000);
    assert_eq!(sw.total_cents, 2_000);

    let expense_total = exp_repo.total_in_range(range).await.unwrap();
    assert_eq!(expense_total, 10_000);

    // Income from the helper SQL the handler uses — sum completed
    // payments in range on paid_at.
    let income: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(amount_cents),0) FROM payments \
         WHERE status='Completed' AND paid_at >= ? AND paid_at < ?",
    )
    .bind(range.start.naive_utc())
    .bind(range.end.naive_utc())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(income, 20_000);

    let net = income - expense_total;
    assert_eq!(net, 10_000);
}

#[tokio::test]
async fn annual_report_aggregates_by_category() {
    // Expenses in category Supplies spread across multiple months
    // sum to a single annual category row.
    let pool = fresh_pool().await;
    let cat_repo: Arc<dyn ExpenseCategoryRepository> =
        Arc::new(SqliteExpenseCategoryRepository::new(pool.clone()));
    let acc_repo: Arc<dyn ExpenseAccountRepository> =
        Arc::new(SqliteExpenseAccountRepository::new(pool.clone()));
    let exp_repo: Arc<dyn ExpenseRepository> = Arc::new(SqliteExpenseRepository::new(pool.clone()));

    let actor = make_member(&pool).await;
    let supplies = cat_repo
        .create(CreateExpenseCategoryRequest {
            name: "Supplies".into(),
            slug: None,
        })
        .await
        .unwrap();
    let acc = acc_repo
        .create(CreateExpenseAccountRequest {
            name: "Card 1".into(),
        })
        .await
        .unwrap();

    for (month, day, amt) in [(1u32, 5, 10_000i64), (4, 15, 20_000), (9, 1, 20_000)] {
        let when = Utc.with_ymd_and_hms(2026, month, day, 12, 0, 0).unwrap();
        exp_repo
            .create(
                actor,
                CreateExpenseRequest {
                    spent_at: when,
                    amount_cents: amt,
                    currency: None,
                    description: "x".into(),
                    category_id: supplies.id,
                    account_id: acc.id,
                    notes: None,
                },
            )
            .await
            .unwrap();
    }

    let range = year_range(2026).await;
    let by_cat = exp_repo.sum_by_category(range).await.unwrap();
    let sup = by_cat.iter().find(|s| s.key_id == supplies.id).unwrap();
    assert_eq!(sup.total_cents, 50_000);
}

#[tokio::test]
async fn date_range_filter_excludes_out_of_range_expenses() {
    let pool = fresh_pool().await;
    let cat_repo: Arc<dyn ExpenseCategoryRepository> =
        Arc::new(SqliteExpenseCategoryRepository::new(pool.clone()));
    let acc_repo: Arc<dyn ExpenseAccountRepository> =
        Arc::new(SqliteExpenseAccountRepository::new(pool.clone()));
    let exp_repo: Arc<dyn ExpenseRepository> = Arc::new(SqliteExpenseRepository::new(pool.clone()));

    let actor = make_member(&pool).await;
    let cat = cat_repo
        .create(CreateExpenseCategoryRequest {
            name: "X".into(),
            slug: None,
        })
        .await
        .unwrap();
    let acc = acc_repo
        .create(CreateExpenseAccountRequest { name: "X".into() })
        .await
        .unwrap();

    // In April
    exp_repo
        .create(
            actor,
            CreateExpenseRequest {
                spent_at: Utc.with_ymd_and_hms(2026, 4, 10, 12, 0, 0).unwrap(),
                amount_cents: 100,
                currency: None,
                description: "in".into(),
                category_id: cat.id,
                account_id: acc.id,
                notes: None,
            },
        )
        .await
        .unwrap();
    // In May (out of range for the April query)
    exp_repo
        .create(
            actor,
            CreateExpenseRequest {
                spent_at: Utc.with_ymd_and_hms(2026, 5, 1, 12, 0, 0).unwrap(),
                amount_cents: 999,
                currency: None,
                description: "out".into(),
                category_id: cat.id,
                account_id: acc.id,
                notes: None,
            },
        )
        .await
        .unwrap();

    let april = month_range(2026, 4).await;
    let total = exp_repo.total_in_range(april).await.unwrap();
    assert_eq!(total, 100, "only the April row counts");
}
