//! Expense-category CRUD handlers — list / new / edit / delete /
//! activate / deactivate.

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
    domain::{CreateExpenseCategoryRequest, UpdateExpenseCategoryRequest},
    service::expense_category_service::ExpenseCategoryService,
    web::{
        portal::admin::partials,
        templates::{BaseContext, HtmlTemplate},
    },
};

#[derive(Debug, Clone)]
pub struct CategoryRow {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub is_active: bool,
}

#[derive(Template)]
#[template(path = "admin/finance/categories_list.html")]
pub struct CategoriesListTemplate {
    pub base: BaseContext,
    pub rows: Vec<CategoryRow>,
}

#[derive(Template)]
#[template(path = "admin/finance/categories_form.html")]
pub struct CategoryFormTemplate {
    pub base: BaseContext,
    pub is_edit: bool,
    pub id: String,
    pub name: String,
    pub slug: String,
    pub is_active: bool,
}

#[derive(Debug, Deserialize)]
pub struct CategoryForm {
    pub name: String,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub is_active: Option<String>,
}

pub async fn list_page(
    State(service): State<Arc<ExpenseCategoryService>>,
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
        .map(|c| CategoryRow {
            id: c.id.to_string(),
            name: c.name,
            slug: c.slug,
            is_active: c.is_active,
        })
        .collect();

    HtmlTemplate(CategoriesListTemplate { base, rows }).into_response()
}

pub async fn new_page(
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> Response {
    let base = BaseContext::for_member(&csrf_service, &current_user, &session_info).await;
    HtmlTemplate(CategoryFormTemplate {
        base,
        is_edit: false,
        id: String::new(),
        name: String::new(),
        slug: String::new(),
        is_active: true,
    })
    .into_response()
}

pub async fn edit_page(
    State(service): State<Arc<ExpenseCategoryService>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Path(id_str): Path<String>,
) -> Response {
    let id = match Uuid::parse_str(&id_str) {
        Ok(v) => v,
        Err(_) => return partials::admin_alert("error", "Invalid id", false).into_response(),
    };
    let category = match service.get(id).await {
        Ok(Some(c)) => c,
        Ok(None) => {
            return partials::admin_alert("error", "Category not found", false).into_response()
        }
        Err(_) => {
            return partials::admin_alert("error", "Error loading category", false).into_response()
        }
    };

    let base = BaseContext::for_member(&csrf_service, &current_user, &session_info).await;
    HtmlTemplate(CategoryFormTemplate {
        base,
        is_edit: true,
        id: category.id.to_string(),
        name: category.name,
        slug: category.slug,
        is_active: category.is_active,
    })
    .into_response()
}

pub async fn create(
    State(service): State<Arc<ExpenseCategoryService>>,
    Extension(current_user): Extension<CurrentUser>,
    axum::Form(form): axum::Form<CategoryForm>,
) -> Response {
    let request = CreateExpenseCategoryRequest {
        name: form.name,
        slug: form.slug.filter(|s| !s.is_empty()),
    };
    match service.create(current_user.member.id, request).await {
        Ok(_) => Redirect::to("/portal/admin/finance/categories").into_response(),
        Err(e) => partials::admin_alert("error", &format!("Error: {}", e), false).into_response(),
    }
}

pub async fn update(
    State(service): State<Arc<ExpenseCategoryService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(id_str): Path<String>,
    axum::Form(form): axum::Form<CategoryForm>,
) -> Response {
    let id = match Uuid::parse_str(&id_str) {
        Ok(v) => v,
        Err(_) => return partials::admin_alert("error", "Invalid id", false).into_response(),
    };
    let request = UpdateExpenseCategoryRequest {
        name: Some(form.name),
        sort_order: None,
        is_active: Some(form.is_active.is_some()),
    };
    match service.update(current_user.member.id, id, request).await {
        Ok(_) => Redirect::to("/portal/admin/finance/categories").into_response(),
        Err(e) => partials::admin_alert("error", &format!("Error: {}", e), false).into_response(),
    }
}

pub async fn delete(
    State(service): State<Arc<ExpenseCategoryService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(id_str): Path<String>,
) -> Response {
    let id = match Uuid::parse_str(&id_str) {
        Ok(v) => v,
        Err(_) => return partials::admin_alert("error", "Invalid id", false).into_response(),
    };
    match service.delete(current_user.member.id, id).await {
        Ok(_) => Redirect::to("/portal/admin/finance/categories").into_response(),
        Err(e) => partials::admin_alert("error", &format!("Error: {}", e), false).into_response(),
    }
}

pub async fn activate(
    State(service): State<Arc<ExpenseCategoryService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(id_str): Path<String>,
) -> Response {
    set_active(&service, current_user.member.id, &id_str, true).await
}

pub async fn deactivate(
    State(service): State<Arc<ExpenseCategoryService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(id_str): Path<String>,
) -> Response {
    set_active(&service, current_user.member.id, &id_str, false).await
}

async fn set_active(
    service: &ExpenseCategoryService,
    actor_id: Uuid,
    id_str: &str,
    is_active: bool,
) -> Response {
    let id = match Uuid::parse_str(id_str) {
        Ok(v) => v,
        Err(_) => return partials::admin_alert("error", "Invalid id", false).into_response(),
    };
    let request = UpdateExpenseCategoryRequest {
        name: None,
        sort_order: None,
        is_active: Some(is_active),
    };
    match service.update(actor_id, id, request).await {
        Ok(_) => Redirect::to("/portal/admin/finance/categories").into_response(),
        Err(e) => partials::admin_alert("error", &format!("Error: {}", e), false).into_response(),
    }
}
