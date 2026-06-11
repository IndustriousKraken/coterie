//! Service layer for the `expense_categories` lookup table.
//!
//! CRUD with audit emission on every mutation. Delete refuses if
//! any `expenses` row still references the category — the operator
//! deactivates (`is_active = 0`) instead. Same shape as
//! [`ExpenseAccountService`].

use std::sync::Arc;

use uuid::Uuid;

use crate::{
    domain::{CreateExpenseCategoryRequest, ExpenseCategory, UpdateExpenseCategoryRequest},
    error::{AppError, Result},
    repository::ExpenseCategoryRepository,
    service::audit_service::AuditService,
};

pub struct ExpenseCategoryService {
    repo: Arc<dyn ExpenseCategoryRepository>,
    audit_service: Arc<AuditService>,
}

impl ExpenseCategoryService {
    pub fn new(repo: Arc<dyn ExpenseCategoryRepository>, audit_service: Arc<AuditService>) -> Self {
        Self {
            repo,
            audit_service,
        }
    }

    pub async fn list(&self, include_inactive: bool) -> Result<Vec<ExpenseCategory>> {
        self.repo.list(include_inactive).await
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<ExpenseCategory>> {
        self.repo.find_by_id(id).await
    }

    pub async fn create(
        &self,
        actor_id: Uuid,
        request: CreateExpenseCategoryRequest,
    ) -> Result<ExpenseCategory> {
        if request.name.trim().is_empty() {
            return Err(AppError::BadRequest("Category name cannot be empty".into()));
        }
        let created = self.repo.create(request).await?;
        self.audit_service
            .log(
                Some(actor_id),
                "create_expense_category",
                "expense_category",
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
        request: UpdateExpenseCategoryRequest,
    ) -> Result<ExpenseCategory> {
        let existing = self
            .repo
            .find_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound("Expense category not found".into()))?;

        if let Some(ref name) = request.name {
            if name.trim().is_empty() {
                return Err(AppError::BadRequest("Category name cannot be empty".into()));
            }
        }

        let old_name = existing.name.clone();
        let updated = self.repo.update(id, request).await?;
        self.audit_service
            .log(
                Some(actor_id),
                "update_expense_category",
                "expense_category",
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
            .ok_or_else(|| AppError::NotFound("Expense category not found".into()))?;

        let usage = self.repo.count_referencing_expenses(id).await?;
        if usage > 0 {
            return Err(AppError::Conflict(format!(
                "Cannot delete category: {} expense row(s) still reference it. Deactivate instead.",
                usage
            )));
        }

        self.repo.delete(id).await?;
        self.audit_service
            .log(
                Some(actor_id),
                "delete_expense_category",
                "expense_category",
                &id.to_string(),
                Some(&existing.name),
                None,
                None,
            )
            .await;
        Ok(())
    }
}
