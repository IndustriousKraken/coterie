//! Expense CRUD handlers (list + new + edit + delete).

use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Redirect, Response},
    Extension,
};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    api::middleware::auth::{CurrentUser, SessionInfo},
    auth::CsrfService,
    domain::{CreateExpenseRequest, ExpenseAccount, ExpenseCategory, UpdateExpenseRequest},
    repository::ExpenseFilter,
    service::{
        expense_account_service::ExpenseAccountService,
        expense_category_service::ExpenseCategoryService, expense_service::ExpenseService,
    },
    web::{
        portal::admin::partials,
        templates::{BaseContext, HtmlTemplate},
    },
};

const PAGE_SIZE: i64 = 50;

#[derive(Debug, Clone)]
pub struct ExpenseRow {
    pub id: String,
    pub spent_at: String,
    pub amount: String,
    pub description: String,
    pub category_name: String,
    pub account_name: String,
}

#[derive(Debug, Clone)]
pub struct CategoryOption {
    pub id: String,
    pub name: String,
    pub is_active: bool,
}

#[derive(Debug, Clone)]
pub struct AccountOption {
    pub id: String,
    pub name: String,
    pub is_active: bool,
}

#[derive(Template)]
#[template(path = "admin/finance/expenses_list.html")]
pub struct ExpensesListTemplate {
    pub base: BaseContext,
    pub rows: Vec<ExpenseRow>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
    pub total_pages: i64,
    pub start_date: String,
    pub end_date: String,
    pub selected_category: String,
    pub selected_account: String,
    pub categories: Vec<CategoryOption>,
    pub accounts: Vec<AccountOption>,
}

#[derive(Template)]
#[template(path = "admin/finance/expense_form.html")]
pub struct ExpenseFormTemplate {
    pub base: BaseContext,
    pub is_edit: bool,
    pub expense_id: String,
    pub spent_at: String,
    pub amount_dollars: String,
    pub description: String,
    pub category_id: String,
    pub account_id: String,
    pub notes: String,
    pub categories: Vec<CategoryOption>,
    pub accounts: Vec<AccountOption>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ExpenseListQuery {
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub end_date: Option<String>,
    #[serde(default)]
    pub category_id: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub page: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ExpenseForm {
    pub spent_at: String,
    pub amount_dollars: String,
    pub description: String,
    pub category_id: String,
    pub account_id: String,
    #[serde(default)]
    pub notes: Option<String>,
}

pub async fn list_page(
    State(expense_service): State<Arc<ExpenseService>>,
    State(category_service): State<Arc<ExpenseCategoryService>>,
    State(account_service): State<Arc<ExpenseAccountService>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Query(query): Query<ExpenseListQuery>,
) -> Response {
    let base = BaseContext::for_member(&csrf_service, &current_user, &session_info).await;

    let start_date_parsed = query.start_date.as_deref().and_then(parse_form_date);
    let end_date_parsed = query.end_date.as_deref().and_then(parse_form_date);
    let date_range = match (start_date_parsed, end_date_parsed) {
        (Some(s), Some(e)) => Some(crate::repository::DateRange {
            start: s,
            end: e + chrono::Duration::days(1),
        }),
        _ => None,
    };

    let category_id_uuid = query
        .category_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .and_then(|s| Uuid::parse_str(s).ok());
    let account_id_uuid = query
        .account_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .and_then(|s| Uuid::parse_str(s).ok());

    let page = query.page.unwrap_or(1).max(1);
    let offset = (page - 1) * PAGE_SIZE;

    let filter = ExpenseFilter {
        date_range,
        category_id: category_id_uuid,
        account_id: account_id_uuid,
        limit: Some(PAGE_SIZE),
        offset: Some(offset),
    };
    let count_filter = ExpenseFilter {
        limit: None,
        offset: None,
        ..filter.clone()
    };

    let expenses = expense_service
        .list_expenses(filter)
        .await
        .unwrap_or_default();
    let total = expense_service
        .count_expenses(count_filter)
        .await
        .unwrap_or(0);

    let categories_all = category_service.list(true).await.unwrap_or_default();
    let accounts_all = account_service.list(true).await.unwrap_or_default();
    let categories: Vec<CategoryOption> = categories_all
        .iter()
        .map(|c| CategoryOption {
            id: c.id.to_string(),
            name: c.name.clone(),
            is_active: c.is_active,
        })
        .collect();
    let accounts: Vec<AccountOption> = accounts_all
        .iter()
        .map(|a| AccountOption {
            id: a.id.to_string(),
            name: a.name.clone(),
            is_active: a.is_active,
        })
        .collect();

    let rows: Vec<ExpenseRow> = expenses
        .into_iter()
        .map(|e| ExpenseRow {
            id: e.id.to_string(),
            spent_at: e.spent_at.format("%Y-%m-%d").to_string(),
            amount: format!("{:.2}", e.amount_cents as f64 / 100.0),
            description: e.description.clone(),
            category_name: categories_all
                .iter()
                .find(|c| c.id == e.category_id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "(unknown)".to_string()),
            account_name: accounts_all
                .iter()
                .find(|a| a.id == e.account_id)
                .map(|a| a.name.clone())
                .unwrap_or_else(|| "(unknown)".to_string()),
        })
        .collect();

    let total_pages = if total == 0 {
        1
    } else {
        (total + PAGE_SIZE - 1) / PAGE_SIZE
    };

    HtmlTemplate(ExpensesListTemplate {
        base,
        rows,
        total,
        page,
        page_size: PAGE_SIZE,
        total_pages,
        start_date: query.start_date.unwrap_or_default(),
        end_date: query.end_date.unwrap_or_default(),
        selected_category: query.category_id.unwrap_or_default(),
        selected_account: query.account_id.unwrap_or_default(),
        categories,
        accounts,
    })
    .into_response()
}

pub async fn new_page(
    State(category_service): State<Arc<ExpenseCategoryService>>,
    State(account_service): State<Arc<ExpenseAccountService>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> Response {
    let base = BaseContext::for_member(&csrf_service, &current_user, &session_info).await;
    let (categories, accounts) = active_dropdowns(&category_service, &account_service).await;
    let today = Utc::now().format("%Y-%m-%d").to_string();

    HtmlTemplate(ExpenseFormTemplate {
        base,
        is_edit: false,
        expense_id: String::new(),
        spent_at: today,
        amount_dollars: String::new(),
        description: String::new(),
        category_id: String::new(),
        account_id: String::new(),
        notes: String::new(),
        categories,
        accounts,
    })
    .into_response()
}

pub async fn edit_page(
    State(expense_service): State<Arc<ExpenseService>>,
    State(category_service): State<Arc<ExpenseCategoryService>>,
    State(account_service): State<Arc<ExpenseAccountService>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Path(expense_id): Path<String>,
) -> Response {
    let id = match Uuid::parse_str(&expense_id) {
        Ok(v) => v,
        Err(_) => {
            return partials::admin_alert("error", "Invalid expense id", false).into_response()
        }
    };
    let expense = match expense_service.get_expense(id).await {
        Ok(Some(e)) => e,
        Ok(None) => {
            return partials::admin_alert("error", "Expense not found", false).into_response()
        }
        Err(_) => {
            return partials::admin_alert("error", "Error loading expense", false).into_response()
        }
    };

    let base = BaseContext::for_member(&csrf_service, &current_user, &session_info).await;
    // Edit dropdowns show *all* categories/accounts (including
    // inactive) so the operator can keep an existing row pinned to
    // the choice it was filed under, even if that lookup row has
    // since been deactivated.
    let cats_all = category_service.list(true).await.unwrap_or_default();
    let accts_all = account_service.list(true).await.unwrap_or_default();
    let categories = cats_all
        .into_iter()
        .map(|c| CategoryOption {
            id: c.id.to_string(),
            name: c.name,
            is_active: c.is_active,
        })
        .collect();
    let accounts = accts_all
        .into_iter()
        .map(|a| AccountOption {
            id: a.id.to_string(),
            name: a.name,
            is_active: a.is_active,
        })
        .collect();

    HtmlTemplate(ExpenseFormTemplate {
        base,
        is_edit: true,
        expense_id: expense.id.to_string(),
        spent_at: expense.spent_at.format("%Y-%m-%d").to_string(),
        amount_dollars: format!("{:.2}", expense.amount_cents as f64 / 100.0),
        description: expense.description,
        category_id: expense.category_id.to_string(),
        account_id: expense.account_id.to_string(),
        notes: expense.notes.unwrap_or_default(),
        categories,
        accounts,
    })
    .into_response()
}

pub async fn create(
    State(expense_service): State<Arc<ExpenseService>>,
    Extension(current_user): Extension<CurrentUser>,
    axum::Form(form): axum::Form<ExpenseForm>,
) -> Response {
    let request = match build_create_request(&form) {
        Ok(r) => r,
        Err(msg) => return partials::admin_alert("error", &msg, false).into_response(),
    };

    match expense_service
        .create_expense(current_user.member.id, request)
        .await
    {
        Ok(_) => Redirect::to("/portal/admin/finance/expenses").into_response(),
        Err(e) => partials::admin_alert("error", &format!("Error: {}", e), false).into_response(),
    }
}

pub async fn update(
    State(expense_service): State<Arc<ExpenseService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(expense_id): Path<String>,
    axum::Form(form): axum::Form<ExpenseForm>,
) -> Response {
    let id = match Uuid::parse_str(&expense_id) {
        Ok(v) => v,
        Err(_) => {
            return partials::admin_alert("error", "Invalid expense id", false).into_response()
        }
    };

    let request = match build_update_request(&form) {
        Ok(r) => r,
        Err(msg) => return partials::admin_alert("error", &msg, false).into_response(),
    };

    match expense_service
        .update_expense(current_user.member.id, id, request)
        .await
    {
        Ok(_) => Redirect::to("/portal/admin/finance/expenses").into_response(),
        Err(e) => partials::admin_alert("error", &format!("Error: {}", e), false).into_response(),
    }
}

pub async fn delete(
    State(expense_service): State<Arc<ExpenseService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(expense_id): Path<String>,
) -> Response {
    let id = match Uuid::parse_str(&expense_id) {
        Ok(v) => v,
        Err(_) => {
            return partials::admin_alert("error", "Invalid expense id", false).into_response()
        }
    };

    match expense_service
        .delete_expense(current_user.member.id, id)
        .await
    {
        Ok(_) => Redirect::to("/portal/admin/finance/expenses").into_response(),
        Err(e) => partials::admin_alert("error", &format!("Error: {}", e), false).into_response(),
    }
}

async fn active_dropdowns(
    category_service: &Arc<ExpenseCategoryService>,
    account_service: &Arc<ExpenseAccountService>,
) -> (Vec<CategoryOption>, Vec<AccountOption>) {
    let cats = category_service.list(false).await.unwrap_or_default();
    let accts = account_service.list(false).await.unwrap_or_default();
    (
        cats.into_iter()
            .map(|c: ExpenseCategory| CategoryOption {
                id: c.id.to_string(),
                name: c.name,
                is_active: c.is_active,
            })
            .collect(),
        accts
            .into_iter()
            .map(|a: ExpenseAccount| AccountOption {
                id: a.id.to_string(),
                name: a.name,
                is_active: a.is_active,
            })
            .collect(),
    )
}

fn build_create_request(form: &ExpenseForm) -> Result<CreateExpenseRequest, String> {
    let spent_at = parse_form_date(&form.spent_at)
        .ok_or_else(|| "Invalid date format (expected YYYY-MM-DD)".to_string())?;
    let amount_cents = parse_dollars_to_cents(&form.amount_dollars)?;
    let category_id =
        Uuid::parse_str(&form.category_id).map_err(|_| "Invalid category".to_string())?;
    let account_id =
        Uuid::parse_str(&form.account_id).map_err(|_| "Invalid account".to_string())?;
    if form.description.trim().is_empty() {
        return Err("Description is required".to_string());
    }

    Ok(CreateExpenseRequest {
        spent_at,
        amount_cents,
        currency: None,
        description: form.description.clone(),
        category_id,
        account_id,
        notes: form
            .notes
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
    })
}

fn build_update_request(form: &ExpenseForm) -> Result<UpdateExpenseRequest, String> {
    let spent_at = parse_form_date(&form.spent_at)
        .ok_or_else(|| "Invalid date format (expected YYYY-MM-DD)".to_string())?;
    let amount_cents = parse_dollars_to_cents(&form.amount_dollars)?;
    let category_id =
        Uuid::parse_str(&form.category_id).map_err(|_| "Invalid category".to_string())?;
    let account_id =
        Uuid::parse_str(&form.account_id).map_err(|_| "Invalid account".to_string())?;
    if form.description.trim().is_empty() {
        return Err("Description is required".to_string());
    }

    Ok(UpdateExpenseRequest {
        spent_at: Some(spent_at),
        amount_cents: Some(amount_cents),
        currency: None,
        description: Some(form.description.clone()),
        category_id: Some(category_id),
        account_id: Some(account_id),
        notes: form
            .notes
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
    })
}

fn parse_form_date(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let date = NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?;
    let datetime = date.and_hms_opt(0, 0, 0)?;
    Utc.from_local_datetime(&datetime).single()
}

fn parse_dollars_to_cents(s: &str) -> Result<i64, String> {
    let dollars: f64 = s
        .trim()
        .parse()
        .map_err(|_| "Amount must be a number".to_string())?;
    if !dollars.is_finite() || dollars < 0.0 {
        return Err("Amount must be non-negative".to_string());
    }
    Ok((dollars * 100.0).round() as i64)
}
