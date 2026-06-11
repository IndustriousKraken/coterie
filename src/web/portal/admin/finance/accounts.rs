//! Expense-account CRUD handlers — same shape as `categories.rs`.

use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Redirect, Response},
    Extension,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    api::middleware::auth::{CurrentUser, SessionInfo},
    auth::CsrfService,
    domain::{CreateExpenseAccountRequest, UpdateExpenseAccountRequest},
    service::expense_account_service::ExpenseAccountService,
    web::{
        portal::admin::partials,
        templates::{BaseContext, HtmlTemplate},
    },
};

#[derive(Debug, Clone)]
pub struct AccountRow {
    pub id: String,
    pub name: String,
    pub is_active: bool,
}

#[derive(Template)]
#[template(path = "admin/finance/accounts_list.html")]
pub struct AccountsListTemplate {
    pub base: BaseContext,
    pub rows: Vec<AccountRow>,
}

#[derive(Template)]
#[template(path = "admin/finance/accounts_form.html")]
pub struct AccountFormTemplate {
    pub base: BaseContext,
    pub is_edit: bool,
    pub id: String,
    pub name: String,
    pub is_active: bool,
}

#[derive(Debug, Deserialize)]
pub struct AccountForm {
    pub name: String,
    #[serde(default)]
    pub is_active: Option<String>,
}

pub async fn list_page(
    State(service): State<Arc<ExpenseAccountService>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> Response {
    let base = BaseContext::for_member(&csrf_service, &current_user, &session_info).await;
    let rows = service
        .list(true)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|a| AccountRow {
            id: a.id.to_string(),
            name: a.name,
            is_active: a.is_active,
        })
        .collect();

    HtmlTemplate(AccountsListTemplate { base, rows }).into_response()
}

pub async fn new_page(
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> Response {
    let base = BaseContext::for_member(&csrf_service, &current_user, &session_info).await;
    HtmlTemplate(AccountFormTemplate {
        base,
        is_edit: false,
        id: String::new(),
        name: String::new(),
        is_active: true,
    })
    .into_response()
}

pub async fn edit_page(
    State(service): State<Arc<ExpenseAccountService>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Path(id_str): Path<String>,
) -> Response {
    let id = match Uuid::parse_str(&id_str) {
        Ok(v) => v,
        Err(_) => return partials::admin_alert("error", "Invalid id", false).into_response(),
    };
    let account = match service.get(id).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return partials::admin_alert("error", "Account not found", false).into_response()
        }
        Err(_) => {
            return partials::admin_alert("error", "Error loading account", false).into_response()
        }
    };

    let base = BaseContext::for_member(&csrf_service, &current_user, &session_info).await;
    HtmlTemplate(AccountFormTemplate {
        base,
        is_edit: true,
        id: account.id.to_string(),
        name: account.name,
        is_active: account.is_active,
    })
    .into_response()
}

pub async fn create(
    State(service): State<Arc<ExpenseAccountService>>,
    Extension(current_user): Extension<CurrentUser>,
    axum::Form(form): axum::Form<AccountForm>,
) -> Response {
    let request = CreateExpenseAccountRequest { name: form.name };
    match service.create(current_user.member.id, request).await {
        Ok(_) => Redirect::to("/portal/admin/finance/accounts").into_response(),
        Err(e) => partials::admin_alert("error", &format!("Error: {}", e), false).into_response(),
    }
}

pub async fn update(
    State(service): State<Arc<ExpenseAccountService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(id_str): Path<String>,
    axum::Form(form): axum::Form<AccountForm>,
) -> Response {
    let id = match Uuid::parse_str(&id_str) {
        Ok(v) => v,
        Err(_) => return partials::admin_alert("error", "Invalid id", false).into_response(),
    };
    let request = UpdateExpenseAccountRequest {
        name: Some(form.name),
        sort_order: None,
        is_active: Some(form.is_active.is_some()),
    };
    match service.update(current_user.member.id, id, request).await {
        Ok(_) => Redirect::to("/portal/admin/finance/accounts").into_response(),
        Err(e) => partials::admin_alert("error", &format!("Error: {}", e), false).into_response(),
    }
}

pub async fn delete(
    State(service): State<Arc<ExpenseAccountService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(id_str): Path<String>,
) -> Response {
    let id = match Uuid::parse_str(&id_str) {
        Ok(v) => v,
        Err(_) => return partials::admin_alert("error", "Invalid id", false).into_response(),
    };
    match service.delete(current_user.member.id, id).await {
        Ok(_) => Redirect::to("/portal/admin/finance/accounts").into_response(),
        Err(e) => partials::admin_alert("error", &format!("Error: {}", e), false).into_response(),
    }
}

pub async fn activate(
    State(service): State<Arc<ExpenseAccountService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(id_str): Path<String>,
) -> Response {
    set_active(&service, current_user.member.id, &id_str, true).await
}

pub async fn deactivate(
    State(service): State<Arc<ExpenseAccountService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(id_str): Path<String>,
) -> Response {
    set_active(&service, current_user.member.id, &id_str, false).await
}

async fn set_active(
    service: &ExpenseAccountService,
    actor_id: Uuid,
    id_str: &str,
    is_active: bool,
) -> Response {
    let id = match Uuid::parse_str(id_str) {
        Ok(v) => v,
        Err(_) => return partials::admin_alert("error", "Invalid id", false).into_response(),
    };
    let request = UpdateExpenseAccountRequest {
        name: None,
        sort_order: None,
        is_active: Some(is_active),
    };
    match service.update(actor_id, id, request).await {
        Ok(_) => Redirect::to("/portal/admin/finance/accounts").into_response(),
        Err(e) => partials::admin_alert("error", &format!("Error: {}", e), false).into_response(),
    }
}
