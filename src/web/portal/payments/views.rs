use std::sync::Arc;

use askama::Template;
use axum::{extract::State, response::IntoResponse, Extension};

use crate::{
    api::middleware::auth::{CurrentUser, SessionInfo},
    auth::CsrfService,
    repository::PaymentRepository,
    web::templates::{BaseContext, HtmlTemplate},
};

#[derive(Template)]
#[template(path = "portal/payments.html")]
pub struct PaymentsTemplate {
    pub base: BaseContext,
}

pub async fn payments_page(
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session): Extension<SessionInfo>,
) -> impl IntoResponse {
    let template = PaymentsTemplate {
        base: BaseContext::for_member(&csrf_service, &current_user, &session).await,
    };

    HtmlTemplate(template)
}

// API endpoint for full payments list
pub async fn payments_list_api(
    State(payment_repo): State<Arc<dyn PaymentRepository>>,
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {
    let payments = payment_repo
        .find_by_member(current_user.member.id)
        .await
        .unwrap_or_default();

    let rows = payments
        .iter()
        .map(crate::web::portal::partials::member_payment_row_from)
        .collect();
    crate::web::portal::partials::member_payment_list(rows)
}

// API endpoint for payments summary
pub async fn payments_summary_api(
    State(payment_repo): State<Arc<dyn PaymentRepository>>,
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {
    use crate::domain::PaymentStatus;

    let payments = payment_repo
        .find_by_member(current_user.member.id)
        .await
        .unwrap_or_default();

    let total: i64 = payments
        .iter()
        .filter(|p| p.status == PaymentStatus::Completed)
        .map(|p| p.amount_cents)
        .sum();

    axum::response::Html(format!("${:.2}", total as f64 / 100.0))
}

// API endpoint for dues status
pub async fn dues_status_api(Extension(current_user): Extension<CurrentUser>) -> impl IntoResponse {
    let status: &'static str = if let Some(dues_until) = current_user.member.dues_paid_until {
        if dues_until > chrono::Utc::now() {
            "current"
        } else {
            "expired"
        }
    } else {
        "unpaid"
    };
    crate::web::portal::partials::dues_status_pill(status)
}

// API endpoint for next due date
pub async fn next_due_api(Extension(current_user): Extension<CurrentUser>) -> impl IntoResponse {
    let next_due = if let Some(dues_until) = current_user.member.dues_paid_until {
        dues_until.format("%B %d, %Y").to_string()
    } else {
        "—".to_string()
    };

    axum::response::Html(next_due)
}
