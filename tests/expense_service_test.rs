//! Service-layer integration tests for `ExpenseService` and its
//! sibling category/account services. Verifies validation,
//! soft-delete-check, and audit emission.

use std::sync::Arc;

use chrono::Utc;
use coterie::{
    domain::{
        CreateExpenseAccountRequest, CreateExpenseCategoryRequest, CreateExpenseRequest,
        UpdateExpenseCategoryRequest, UpdateExpenseRequest, MAX_EXPENSE_CENTS,
    },
    repository::{
        ExpenseAccountRepository, ExpenseCategoryRepository, ExpenseRepository,
        SqliteExpenseAccountRepository, SqliteExpenseCategoryRepository, SqliteExpenseRepository,
    },
    service::{
        audit_service::AuditService, expense_account_service::ExpenseAccountService,
        expense_category_service::ExpenseCategoryService, expense_service::ExpenseService,
    },
};
use uuid::Uuid;

mod common;
use common::{fresh_pool, make_member};

struct Setup {
    pool: sqlx::SqlitePool,
    expense_service: ExpenseService,
    category_service: ExpenseCategoryService,
    account_service: ExpenseAccountService,
}

async fn setup() -> Setup {
    let pool = fresh_pool().await;
    let audit = Arc::new(AuditService::new(pool.clone()));
    let cat_repo: Arc<dyn ExpenseCategoryRepository> =
        Arc::new(SqliteExpenseCategoryRepository::new(pool.clone()));
    let acc_repo: Arc<dyn ExpenseAccountRepository> =
        Arc::new(SqliteExpenseAccountRepository::new(pool.clone()));
    let exp_repo: Arc<dyn ExpenseRepository> = Arc::new(SqliteExpenseRepository::new(pool.clone()));

    let expense_service =
        ExpenseService::new(exp_repo, cat_repo.clone(), acc_repo.clone(), audit.clone());
    let category_service = ExpenseCategoryService::new(cat_repo, audit.clone());
    let account_service = ExpenseAccountService::new(acc_repo, audit);
    Setup {
        pool,
        expense_service,
        category_service,
        account_service,
    }
}

async fn seed_cat_and_acc(s: &Setup, actor: Uuid) -> (Uuid, Uuid) {
    let cat = s
        .category_service
        .create(
            actor,
            CreateExpenseCategoryRequest {
                name: "Supplies".to_string(),
                slug: None,
            },
        )
        .await
        .unwrap();
    let acc = s
        .account_service
        .create(
            actor,
            CreateExpenseAccountRequest {
                name: "Card 1".to_string(),
            },
        )
        .await
        .unwrap();
    (cat.id, acc.id)
}

async fn count_audit(pool: &sqlx::SqlitePool, action: &str) -> i64 {
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM audit_logs WHERE action = ?")
        .bind(action)
        .fetch_one(pool)
        .await
        .unwrap();
    n
}

#[tokio::test]
async fn create_expense_audits_and_inserts() {
    let s = setup().await;
    let actor = make_member(&s.pool).await;
    let (cat_id, acc_id) = seed_cat_and_acc(&s, actor).await;

    let created = s
        .expense_service
        .create_expense(
            actor,
            CreateExpenseRequest {
                spent_at: Utc::now(),
                amount_cents: 3_000,
                currency: None,
                description: "Sticky notes".into(),
                category_id: cat_id,
                account_id: acc_id,
                notes: None,
            },
        )
        .await
        .expect("create expense");

    assert_eq!(created.amount_cents, 3_000);
    assert_eq!(count_audit(&s.pool, "create_expense").await, 1);

    // Audit row's new_value carries the summary string.
    let (new_value,): (String,) = sqlx::query_as(
        "SELECT new_value FROM audit_logs WHERE action = 'create_expense' AND entity_id = ?",
    )
    .bind(created.id.to_string())
    .fetch_one(&s.pool)
    .await
    .unwrap();
    assert!(new_value.contains("$30.00"));
    assert!(new_value.contains("Supplies"));
    assert!(new_value.contains("Card 1"));
}

#[tokio::test]
async fn create_expense_rejects_negative_amount() {
    let s = setup().await;
    let actor = make_member(&s.pool).await;
    let (cat_id, acc_id) = seed_cat_and_acc(&s, actor).await;

    let err = s
        .expense_service
        .create_expense(
            actor,
            CreateExpenseRequest {
                spent_at: Utc::now(),
                amount_cents: -100,
                currency: None,
                description: "Refund-shaped".into(),
                category_id: cat_id,
                account_id: acc_id,
                notes: None,
            },
        )
        .await
        .err();
    assert!(err.is_some(), "negative amount must be refused");

    // And no row was inserted.
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM expenses")
        .fetch_one(&s.pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn create_expense_rejects_too_large_amount() {
    let s = setup().await;
    let actor = make_member(&s.pool).await;
    let (cat_id, acc_id) = seed_cat_and_acc(&s, actor).await;

    let err = s
        .expense_service
        .create_expense(
            actor,
            CreateExpenseRequest {
                spent_at: Utc::now(),
                amount_cents: MAX_EXPENSE_CENTS + 1,
                currency: None,
                description: "Suspicious".into(),
                category_id: cat_id,
                account_id: acc_id,
                notes: None,
            },
        )
        .await
        .err();
    assert!(err.is_some(), "above-ceiling amount must be refused");
}

#[tokio::test]
async fn create_expense_rejects_inactive_category() {
    let s = setup().await;
    let actor = make_member(&s.pool).await;
    let (cat_id, acc_id) = seed_cat_and_acc(&s, actor).await;

    // Deactivate the category first.
    s.category_service
        .update(
            actor,
            cat_id,
            UpdateExpenseCategoryRequest {
                is_active: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let err = s
        .expense_service
        .create_expense(
            actor,
            CreateExpenseRequest {
                spent_at: Utc::now(),
                amount_cents: 100,
                currency: None,
                description: "x".into(),
                category_id: cat_id,
                account_id: acc_id,
                notes: None,
            },
        )
        .await
        .err();
    assert!(err.is_some(), "inactive category must be refused");
}

#[tokio::test]
async fn create_expense_rejects_empty_description() {
    let s = setup().await;
    let actor = make_member(&s.pool).await;
    let (cat_id, acc_id) = seed_cat_and_acc(&s, actor).await;

    let err = s
        .expense_service
        .create_expense(
            actor,
            CreateExpenseRequest {
                spent_at: Utc::now(),
                amount_cents: 100,
                currency: None,
                description: "   ".into(),
                category_id: cat_id,
                account_id: acc_id,
                notes: None,
            },
        )
        .await
        .err();
    assert!(err.is_some(), "blank description must be refused");
}

#[tokio::test]
async fn delete_category_with_existing_expenses_refuses() {
    let s = setup().await;
    let actor = make_member(&s.pool).await;
    let (cat_id, acc_id) = seed_cat_and_acc(&s, actor).await;

    s.expense_service
        .create_expense(
            actor,
            CreateExpenseRequest {
                spent_at: Utc::now(),
                amount_cents: 100,
                currency: None,
                description: "x".into(),
                category_id: cat_id,
                account_id: acc_id,
                notes: None,
            },
        )
        .await
        .unwrap();

    let err = s.category_service.delete(actor, cat_id).await.err();
    assert!(
        err.is_some(),
        "delete must refuse when expenses reference category"
    );

    // The category is still there.
    assert!(s.category_service.get(cat_id).await.unwrap().is_some());
}

#[tokio::test]
async fn delete_account_with_existing_expenses_refuses() {
    let s = setup().await;
    let actor = make_member(&s.pool).await;
    let (cat_id, acc_id) = seed_cat_and_acc(&s, actor).await;

    s.expense_service
        .create_expense(
            actor,
            CreateExpenseRequest {
                spent_at: Utc::now(),
                amount_cents: 100,
                currency: None,
                description: "x".into(),
                category_id: cat_id,
                account_id: acc_id,
                notes: None,
            },
        )
        .await
        .unwrap();

    let err = s.account_service.delete(actor, acc_id).await.err();
    assert!(
        err.is_some(),
        "delete must refuse when expenses reference account"
    );
    assert!(s.account_service.get(acc_id).await.unwrap().is_some());
}

#[tokio::test]
async fn update_expense_emits_audit_with_old_and_new() {
    let s = setup().await;
    let actor = make_member(&s.pool).await;
    let (cat_id, acc_id) = seed_cat_and_acc(&s, actor).await;

    let created = s
        .expense_service
        .create_expense(
            actor,
            CreateExpenseRequest {
                spent_at: Utc::now(),
                amount_cents: 1_000,
                currency: None,
                description: "Original".into(),
                category_id: cat_id,
                account_id: acc_id,
                notes: None,
            },
        )
        .await
        .unwrap();

    s.expense_service
        .update_expense(
            actor,
            created.id,
            UpdateExpenseRequest {
                amount_cents: Some(5_000),
                description: Some("Updated".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let (old, new): (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT old_value, new_value FROM audit_logs WHERE action = 'update_expense' AND entity_id = ?",
    )
    .bind(created.id.to_string())
    .fetch_one(&s.pool)
    .await
    .unwrap();

    let old = old.expect("old_value present");
    let new = new.expect("new_value present");
    assert!(
        old.contains("$10.00"),
        "old summary should mention pre-change amount"
    );
    assert!(old.contains("Original"));
    assert!(
        new.contains("$50.00"),
        "new summary should mention post-change amount"
    );
    assert!(new.contains("Updated"));
}

#[tokio::test]
async fn delete_expense_emits_audit_and_removes_row() {
    let s = setup().await;
    let actor = make_member(&s.pool).await;
    let (cat_id, acc_id) = seed_cat_and_acc(&s, actor).await;

    let created = s
        .expense_service
        .create_expense(
            actor,
            CreateExpenseRequest {
                spent_at: Utc::now(),
                amount_cents: 100,
                currency: None,
                description: "Bye".into(),
                category_id: cat_id,
                account_id: acc_id,
                notes: None,
            },
        )
        .await
        .unwrap();

    s.expense_service
        .delete_expense(actor, created.id)
        .await
        .unwrap();

    assert!(s
        .expense_service
        .get_expense(created.id)
        .await
        .unwrap()
        .is_none());
    assert_eq!(count_audit(&s.pool, "delete_expense").await, 1);
}
