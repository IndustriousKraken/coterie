//! Service layer for the expense ledger.
//!
//! Each mutation validates at the service boundary (amount in range,
//! description non-empty, referenced category + account exist AND
//! are active), writes via the repo, then emits an audit row through
//! [`AuditService`]. Audit emission is fire-and-forget — see
//! `AuditService::log`.

use std::sync::Arc;

use uuid::Uuid;

use crate::{
    domain::{
        CreateExpenseRequest, Expense, ExpenseAccount, ExpenseCategory, UpdateExpenseRequest,
        MAX_EXPENSE_CENTS,
    },
    error::{AppError, Result},
    repository::{
        ExpenseAccountRepository, ExpenseCategoryRepository, ExpenseFilter, ExpenseRepository,
    },
    service::audit_service::AuditService,
};

pub struct ExpenseService {
    expense_repo: Arc<dyn ExpenseRepository>,
    category_repo: Arc<dyn ExpenseCategoryRepository>,
    account_repo: Arc<dyn ExpenseAccountRepository>,
    audit_service: Arc<AuditService>,
}

impl ExpenseService {
    pub fn new(
        expense_repo: Arc<dyn ExpenseRepository>,
        category_repo: Arc<dyn ExpenseCategoryRepository>,
        account_repo: Arc<dyn ExpenseAccountRepository>,
        audit_service: Arc<AuditService>,
    ) -> Self {
        Self {
            expense_repo,
            category_repo,
            account_repo,
            audit_service,
        }
    }

    pub async fn get_expense(&self, id: Uuid) -> Result<Option<Expense>> {
        self.expense_repo.find_by_id(id).await
    }

    pub async fn list_expenses(&self, filter: ExpenseFilter) -> Result<Vec<Expense>> {
        self.expense_repo.list(filter).await
    }

    pub async fn count_expenses(&self, filter: ExpenseFilter) -> Result<i64> {
        self.expense_repo.count(filter).await
    }

    pub async fn create_expense(
        &self,
        actor_id: Uuid,
        request: CreateExpenseRequest,
    ) -> Result<Expense> {
        validate_amount(request.amount_cents)?;
        validate_description(&request.description)?;
        let category = self.require_active_category(request.category_id).await?;
        let account = self.require_active_account(request.account_id).await?;

        let summary = expense_summary_text(
            request.amount_cents,
            &request.description,
            &category.name,
            &account.name,
            request.spent_at,
        );

        let expense = self.expense_repo.create(actor_id, request).await?;

        self.audit_service
            .log(
                Some(actor_id),
                "create_expense",
                "expense",
                &expense.id.to_string(),
                None,
                Some(&summary),
                None,
            )
            .await;

        Ok(expense)
    }

    pub async fn update_expense(
        &self,
        actor_id: Uuid,
        id: Uuid,
        request: UpdateExpenseRequest,
    ) -> Result<Expense> {
        let existing = self
            .expense_repo
            .find_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound("Expense not found".into()))?;

        // Validate any fields the caller actually wants to change. Fields
        // they didn't touch keep the existing row's value, which already
        // passed validation on insert.
        if let Some(amount) = request.amount_cents {
            validate_amount(amount)?;
        }
        if let Some(desc) = &request.description {
            validate_description(desc)?;
        }
        if let Some(cat_id) = request.category_id {
            if cat_id != existing.category_id {
                self.require_active_category(cat_id).await?;
            }
        }
        if let Some(acc_id) = request.account_id {
            if acc_id != existing.account_id {
                self.require_active_account(acc_id).await?;
            }
        }

        let old_summary = self.summary_for_expense(&existing).await;
        let updated = self.expense_repo.update(id, request).await?;
        let new_summary = self.summary_for_expense(&updated).await;

        self.audit_service
            .log(
                Some(actor_id),
                "update_expense",
                "expense",
                &id.to_string(),
                Some(&old_summary),
                Some(&new_summary),
                None,
            )
            .await;

        Ok(updated)
    }

    pub async fn delete_expense(&self, actor_id: Uuid, id: Uuid) -> Result<()> {
        let existing = self
            .expense_repo
            .find_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound("Expense not found".into()))?;

        let summary = self.summary_for_expense(&existing).await;

        self.expense_repo.delete(id).await?;

        self.audit_service
            .log(
                Some(actor_id),
                "delete_expense",
                "expense",
                &id.to_string(),
                Some(&summary),
                None,
                None,
            )
            .await;

        Ok(())
    }

    async fn require_active_category(&self, id: Uuid) -> Result<ExpenseCategory> {
        let category = self
            .category_repo
            .find_by_id(id)
            .await?
            .ok_or_else(|| AppError::BadRequest("Unknown expense category".into()))?;
        if !category.is_active {
            return Err(AppError::BadRequest(
                "Selected expense category is inactive".into(),
            ));
        }
        Ok(category)
    }

    async fn require_active_account(&self, id: Uuid) -> Result<ExpenseAccount> {
        let account = self
            .account_repo
            .find_by_id(id)
            .await?
            .ok_or_else(|| AppError::BadRequest("Unknown expense account".into()))?;
        if !account.is_active {
            return Err(AppError::BadRequest(
                "Selected expense account is inactive".into(),
            ));
        }
        Ok(account)
    }

    /// Build the human-readable audit summary string for an expense
    /// row by resolving its category + account names. Falls back to
    /// "?" when a lookup fails — preserves an unbroken audit trail
    /// even if a referenced row was hard-deleted out of band.
    async fn summary_for_expense(&self, expense: &Expense) -> String {
        let category_name = match self.category_repo.find_by_id(expense.category_id).await {
            Ok(Some(c)) => c.name,
            _ => "?".to_string(),
        };
        let account_name = match self.account_repo.find_by_id(expense.account_id).await {
            Ok(Some(a)) => a.name,
            _ => "?".to_string(),
        };
        expense_summary_text(
            expense.amount_cents,
            &expense.description,
            &category_name,
            &account_name,
            expense.spent_at,
        )
    }
}

fn validate_amount(amount_cents: i64) -> Result<()> {
    if amount_cents < 0 {
        return Err(AppError::BadRequest(
            "Expense amount cannot be negative".into(),
        ));
    }
    if amount_cents > MAX_EXPENSE_CENTS {
        return Err(AppError::BadRequest(format!(
            "Expense amount exceeds the ${:.2} ceiling",
            MAX_EXPENSE_CENTS as f64 / 100.0
        )));
    }
    Ok(())
}

fn validate_description(description: &str) -> Result<()> {
    if description.trim().is_empty() {
        return Err(AppError::BadRequest(
            "Expense description cannot be empty".into(),
        ));
    }
    Ok(())
}

fn expense_summary_text(
    amount_cents: i64,
    description: &str,
    category_name: &str,
    account_name: &str,
    spent_at: chrono::DateTime<chrono::Utc>,
) -> String {
    format!(
        "${:.2} / {} / {} / {} / {}",
        amount_cents as f64 / 100.0,
        description,
        category_name,
        account_name,
        spent_at.format("%Y-%m-%d"),
    )
}
