//! Integration tests for the three expense repos:
//!   * `ExpenseRepository` (CRUD + filter + sum aggregations)
//!   * `ExpenseCategoryRepository` (CRUD + referential count)
//!   * `ExpenseAccountRepository` (CRUD + referential count)

use std::sync::Arc;

use chrono::{Duration, TimeZone, Utc};
use coterie::{
    domain::{
        CreateExpenseAccountRequest, CreateExpenseCategoryRequest, CreateExpenseRequest,
        UpdateExpenseAccountRequest, UpdateExpenseCategoryRequest, UpdateExpenseRequest,
    },
    repository::{
        expense_repository::DateRange, ExpenseAccountRepository, ExpenseCategoryRepository,
        ExpenseFilter, ExpenseRepository, SqliteExpenseAccountRepository,
        SqliteExpenseCategoryRepository, SqliteExpenseRepository,
    },
};
use uuid::Uuid;

mod common;
use common::{fresh_pool, make_member};

async fn seed_category(
    repo: &dyn ExpenseCategoryRepository,
    name: &str,
) -> coterie::domain::ExpenseCategory {
    repo.create(CreateExpenseCategoryRequest {
        name: name.to_string(),
        slug: None,
    })
    .await
    .expect("create category")
}

async fn seed_account(
    repo: &dyn ExpenseAccountRepository,
    name: &str,
) -> coterie::domain::ExpenseAccount {
    repo.create(CreateExpenseAccountRequest {
        name: name.to_string(),
    })
    .await
    .expect("create account")
}

#[tokio::test]
async fn expense_create_then_find_round_trips_every_field() {
    let pool = fresh_pool().await;
    let cat_repo: Arc<dyn ExpenseCategoryRepository> =
        Arc::new(SqliteExpenseCategoryRepository::new(pool.clone()));
    let acc_repo: Arc<dyn ExpenseAccountRepository> =
        Arc::new(SqliteExpenseAccountRepository::new(pool.clone()));
    let exp_repo: Arc<dyn ExpenseRepository> = Arc::new(SqliteExpenseRepository::new(pool.clone()));

    let actor = make_member(&pool).await;
    let cat = seed_category(cat_repo.as_ref(), "Supplies").await;
    let acc = seed_account(acc_repo.as_ref(), "Card 1").await;

    let spent_at = Utc.with_ymd_and_hms(2026, 3, 15, 0, 0, 0).unwrap();
    let created = exp_repo
        .create(
            actor,
            CreateExpenseRequest {
                spent_at,
                amount_cents: 3_000,
                currency: None,
                description: "Sticky notes".to_string(),
                category_id: cat.id,
                account_id: acc.id,
                notes: Some("Bulk pack".to_string()),
            },
        )
        .await
        .expect("create expense");

    let fetched = exp_repo
        .find_by_id(created.id)
        .await
        .expect("find")
        .expect("present");

    assert_eq!(fetched.spent_at, spent_at);
    assert_eq!(fetched.amount_cents, 3_000);
    assert_eq!(fetched.currency, "USD");
    assert_eq!(fetched.description, "Sticky notes");
    assert_eq!(fetched.category_id, cat.id);
    assert_eq!(fetched.account_id, acc.id);
    assert_eq!(fetched.notes.as_deref(), Some("Bulk pack"));
    assert_eq!(fetched.created_by, actor);
}

#[tokio::test]
async fn expense_update_changes_only_supplied_fields() {
    let pool = fresh_pool().await;
    let cat_repo: Arc<dyn ExpenseCategoryRepository> =
        Arc::new(SqliteExpenseCategoryRepository::new(pool.clone()));
    let acc_repo: Arc<dyn ExpenseAccountRepository> =
        Arc::new(SqliteExpenseAccountRepository::new(pool.clone()));
    let exp_repo: Arc<dyn ExpenseRepository> = Arc::new(SqliteExpenseRepository::new(pool.clone()));

    let actor = make_member(&pool).await;
    let cat = seed_category(cat_repo.as_ref(), "Supplies").await;
    let acc = seed_account(acc_repo.as_ref(), "Card 1").await;

    let created = exp_repo
        .create(
            actor,
            CreateExpenseRequest {
                spent_at: Utc::now(),
                amount_cents: 100,
                currency: None,
                description: "First".to_string(),
                category_id: cat.id,
                account_id: acc.id,
                notes: None,
            },
        )
        .await
        .unwrap();

    let updated = exp_repo
        .update(
            created.id,
            UpdateExpenseRequest {
                amount_cents: Some(500),
                description: Some("Second".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(updated.amount_cents, 500);
    assert_eq!(updated.description, "Second");
    // Untouched fields stay as they were
    assert_eq!(updated.category_id, cat.id);
    assert_eq!(updated.account_id, acc.id);
}

#[tokio::test]
async fn expense_delete_removes_row() {
    let pool = fresh_pool().await;
    let cat_repo: Arc<dyn ExpenseCategoryRepository> =
        Arc::new(SqliteExpenseCategoryRepository::new(pool.clone()));
    let acc_repo: Arc<dyn ExpenseAccountRepository> =
        Arc::new(SqliteExpenseAccountRepository::new(pool.clone()));
    let exp_repo: Arc<dyn ExpenseRepository> = Arc::new(SqliteExpenseRepository::new(pool.clone()));

    let actor = make_member(&pool).await;
    let cat = seed_category(cat_repo.as_ref(), "Cat").await;
    let acc = seed_account(acc_repo.as_ref(), "Acc").await;

    let created = exp_repo
        .create(
            actor,
            CreateExpenseRequest {
                spent_at: Utc::now(),
                amount_cents: 50,
                currency: None,
                description: "Doomed".to_string(),
                category_id: cat.id,
                account_id: acc.id,
                notes: None,
            },
        )
        .await
        .unwrap();

    exp_repo.delete(created.id).await.unwrap();
    assert!(exp_repo.find_by_id(created.id).await.unwrap().is_none());
}

#[tokio::test]
async fn expense_list_filters_by_date_range_and_returns_descending() {
    let pool = fresh_pool().await;
    let cat_repo: Arc<dyn ExpenseCategoryRepository> =
        Arc::new(SqliteExpenseCategoryRepository::new(pool.clone()));
    let acc_repo: Arc<dyn ExpenseAccountRepository> =
        Arc::new(SqliteExpenseAccountRepository::new(pool.clone()));
    let exp_repo: Arc<dyn ExpenseRepository> = Arc::new(SqliteExpenseRepository::new(pool.clone()));

    let actor = make_member(&pool).await;
    let cat = seed_category(cat_repo.as_ref(), "Cat").await;
    let acc = seed_account(acc_repo.as_ref(), "Acc").await;

    let day = |y, m, d| Utc.with_ymd_and_hms(y, m, d, 12, 0, 0).unwrap();
    for d in [day(2026, 3, 1), day(2026, 3, 15), day(2026, 4, 5)] {
        exp_repo
            .create(
                actor,
                CreateExpenseRequest {
                    spent_at: d,
                    amount_cents: 100,
                    currency: None,
                    description: format!("d-{}", d.format("%Y-%m-%d")),
                    category_id: cat.id,
                    account_id: acc.id,
                    notes: None,
                },
            )
            .await
            .unwrap();
    }

    let march_only = ExpenseFilter {
        date_range: Some(DateRange {
            start: day(2026, 3, 1) - Duration::hours(1),
            end: day(2026, 4, 1),
        }),
        ..Default::default()
    };
    let rows = exp_repo.list(march_only).await.unwrap();
    assert_eq!(rows.len(), 2);
    // DESC by spent_at — March 15 first.
    assert!(rows[0].spent_at > rows[1].spent_at);
}

#[tokio::test]
async fn expense_sum_by_account_and_by_category_match_inserts() {
    let pool = fresh_pool().await;
    let cat_repo: Arc<dyn ExpenseCategoryRepository> =
        Arc::new(SqliteExpenseCategoryRepository::new(pool.clone()));
    let acc_repo: Arc<dyn ExpenseAccountRepository> =
        Arc::new(SqliteExpenseAccountRepository::new(pool.clone()));
    let exp_repo: Arc<dyn ExpenseRepository> = Arc::new(SqliteExpenseRepository::new(pool.clone()));

    let actor = make_member(&pool).await;
    let supplies = seed_category(cat_repo.as_ref(), "Supplies").await;
    let software = seed_category(cat_repo.as_ref(), "Software").await;
    let card1 = seed_account(acc_repo.as_ref(), "Card 1").await;

    let d = Utc.with_ymd_and_hms(2026, 5, 10, 12, 0, 0).unwrap();
    for (amt, cat) in [
        (3_000i64, supplies.id),
        (5_000, supplies.id),
        (2_000, software.id),
    ] {
        exp_repo
            .create(
                actor,
                CreateExpenseRequest {
                    spent_at: d,
                    amount_cents: amt,
                    currency: None,
                    description: "x".to_string(),
                    category_id: cat,
                    account_id: card1.id,
                    notes: None,
                },
            )
            .await
            .unwrap();
    }

    let range = DateRange {
        start: Utc.with_ymd_and_hms(2026, 5, 1, 0, 0, 0).unwrap(),
        end: Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap(),
    };
    let by_acct = exp_repo.sum_by_account(range).await.unwrap();
    assert_eq!(by_acct.len(), 1);
    assert_eq!(by_acct[0].key_id, card1.id);
    assert_eq!(by_acct[0].total_cents, 10_000);

    let by_cat = exp_repo.sum_by_category(range).await.unwrap();
    let s = by_cat
        .iter()
        .find(|s| s.key_id == supplies.id)
        .expect("supplies row");
    assert_eq!(s.total_cents, 8_000);
    let sw = by_cat
        .iter()
        .find(|s| s.key_id == software.id)
        .expect("software row");
    assert_eq!(sw.total_cents, 2_000);
}

// =============================================================================
// Category repo
// =============================================================================

#[tokio::test]
async fn category_count_referencing_expenses_tracks_inserts() {
    let pool = fresh_pool().await;
    let cat_repo: Arc<dyn ExpenseCategoryRepository> =
        Arc::new(SqliteExpenseCategoryRepository::new(pool.clone()));
    let acc_repo: Arc<dyn ExpenseAccountRepository> =
        Arc::new(SqliteExpenseAccountRepository::new(pool.clone()));
    let exp_repo: Arc<dyn ExpenseRepository> = Arc::new(SqliteExpenseRepository::new(pool.clone()));

    let actor = make_member(&pool).await;
    let cat = seed_category(cat_repo.as_ref(), "Cat").await;
    let acc = seed_account(acc_repo.as_ref(), "Acc").await;

    assert_eq!(
        cat_repo.count_referencing_expenses(cat.id).await.unwrap(),
        0
    );

    exp_repo
        .create(
            actor,
            CreateExpenseRequest {
                spent_at: Utc::now(),
                amount_cents: 1,
                currency: None,
                description: "a".into(),
                category_id: cat.id,
                account_id: acc.id,
                notes: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(
        cat_repo.count_referencing_expenses(cat.id).await.unwrap(),
        1
    );
}

#[tokio::test]
async fn category_list_excludes_inactive_when_requested() {
    let pool = fresh_pool().await;
    let cat_repo: Arc<dyn ExpenseCategoryRepository> =
        Arc::new(SqliteExpenseCategoryRepository::new(pool.clone()));

    let a = seed_category(cat_repo.as_ref(), "Active").await;
    let i = seed_category(cat_repo.as_ref(), "Inactive").await;
    cat_repo
        .update(
            i.id,
            UpdateExpenseCategoryRequest {
                is_active: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let active_only = cat_repo.list(false).await.unwrap();
    assert!(active_only.iter().any(|c| c.id == a.id));
    assert!(!active_only.iter().any(|c| c.id == i.id));

    let all = cat_repo.list(true).await.unwrap();
    assert!(all.iter().any(|c| c.id == i.id));
}

// =============================================================================
// Account repo
// =============================================================================

#[tokio::test]
async fn account_create_update_delete_round_trips() {
    let pool = fresh_pool().await;
    let acc_repo: Arc<dyn ExpenseAccountRepository> =
        Arc::new(SqliteExpenseAccountRepository::new(pool.clone()));

    let created = seed_account(acc_repo.as_ref(), "Card 1").await;
    let updated = acc_repo
        .update(
            created.id,
            UpdateExpenseAccountRequest {
                name: Some("Card 1 – Jane".into()),
                is_active: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.name, "Card 1 – Jane");
    assert!(!updated.is_active);

    // No referencing expense → delete succeeds.
    assert_eq!(
        acc_repo
            .count_referencing_expenses(created.id)
            .await
            .unwrap(),
        0
    );
    acc_repo.delete(created.id).await.unwrap();
    assert!(acc_repo.find_by_id(created.id).await.unwrap().is_none());
}

#[tokio::test]
async fn account_count_referencing_expenses_picks_up_inserts() {
    let pool = fresh_pool().await;
    let cat_repo: Arc<dyn ExpenseCategoryRepository> =
        Arc::new(SqliteExpenseCategoryRepository::new(pool.clone()));
    let acc_repo: Arc<dyn ExpenseAccountRepository> =
        Arc::new(SqliteExpenseAccountRepository::new(pool.clone()));
    let exp_repo: Arc<dyn ExpenseRepository> = Arc::new(SqliteExpenseRepository::new(pool.clone()));

    let actor = make_member(&pool).await;
    let cat = seed_category(cat_repo.as_ref(), "Cat").await;
    let acc = seed_account(acc_repo.as_ref(), "Acc").await;

    exp_repo
        .create(
            actor,
            CreateExpenseRequest {
                spent_at: Utc::now(),
                amount_cents: 1,
                currency: None,
                description: "a".into(),
                category_id: cat.id,
                account_id: acc.id,
                notes: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(
        acc_repo.count_referencing_expenses(acc.id).await.unwrap(),
        1
    );
}

#[tokio::test]
async fn account_create_rejects_duplicate_name() {
    let pool = fresh_pool().await;
    let acc_repo: Arc<dyn ExpenseAccountRepository> =
        Arc::new(SqliteExpenseAccountRepository::new(pool.clone()));

    seed_account(acc_repo.as_ref(), "Card 1").await;
    let err = acc_repo
        .create(CreateExpenseAccountRequest {
            name: "Card 1".to_string(),
        })
        .await
        .err();
    assert!(err.is_some(), "duplicate name must be rejected");
}

// silence unused-warning for the dead-code Uuid import on some configs.
#[allow(dead_code)]
fn _ensure_uuid_in_use(_: Uuid) {}
