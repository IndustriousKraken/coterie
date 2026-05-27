//! Service layer for the `expense_accounts` lookup table.
//!
//! Same shape as [`ExpenseCategoryService`]: CRUD with audit emission,
//! and delete refuses if any `expenses` row references the account.

use std::sync::Arc;

use uuid::Uuid;

use crate::{
    domain::{CreateExpenseAccountRequest, ExpenseAccount, UpdateExpenseAccountRequest},
    error::{AppError, Result},
    repository::ExpenseAccountRepository,
    service::audit_service::AuditService,
};

pub struct ExpenseAccountService {
    repo: Arc<dyn ExpenseAccountRepository>,
    audit_service: Arc<AuditService>,
}

impl ExpenseAccountService {
    pub fn new(repo: Arc<dyn ExpenseAccountRepository>, audit_service: Arc<AuditService>) -> Self {
        Self {
            repo,
            audit_service,
        }
    }

    pub async fn list(&self, include_inactive: bool) -> Result<Vec<ExpenseAccount>> {
        self.repo.list(include_inactive).await
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<ExpenseAccount>> {
        self.repo.find_by_id(id).await
    }

    pub async fn create(
        &self,
        actor_id: Uuid,
        request: CreateExpenseAccountRequest,
    ) -> Result<ExpenseAccount> {
        if request.name.trim().is_empty() {
            return Err(AppError::BadRequest("Account name cannot be empty".into()));
        }
        let created = self.repo.create(request).await?;
        self.audit_service
            .log(
                Some(actor_id),
                "create_expense_account",
                "expense_account",
                &created.id.to_string(),
                None,
                Some(&created.name),
                None,
            )
            .await;
        Ok(created)
    }

    pub async fn update(
        &self,
        actor_id: Uuid,
        id: Uuid,
        request: UpdateExpenseAccountRequest,
    ) -> Result<ExpenseAccount> {
        let existing = self
            .repo
            .find_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound("Expense account not found".into()))?;

        if let Some(ref name) = request.name {
            if name.trim().is_empty() {
                return Err(AppError::BadRequest("Account name cannot be empty".into()));
            }
        }

        let old_name = existing.name.clone();
        let updated = self.repo.update(id, request).await?;
        self.audit_service
            .log(
                Some(actor_id),
                "update_expense_account",
                "expense_account",
                &id.to_string(),
                Some(&old_name),
                Some(&updated.name),
                None,
            )
            .await;
        Ok(updated)
    }

    pub async fn delete(&self, actor_id: Uuid, id: Uuid) -> Result<()> {
        let existing = self
            .repo
            .find_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound("Expense account not found".into()))?;

        let usage = self.repo.count_referencing_expenses(id).await?;
        if usage > 0 {
            return Err(AppError::Conflict(format!(
                "Cannot delete account: {} expense row(s) still reference it. Deactivate instead.",
                usage
            )));
        }

        self.repo.delete(id).await?;
        self.audit_service
            .log(
                Some(actor_id),
                "delete_expense_account",
                "expense_account",
                &id.to_string(),
                Some(&existing.name),
                None,
                None,
            )
            .await;
        Ok(())
    }
}
