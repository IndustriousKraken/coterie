use askama::Template;
use axum::{
    extract::State,
    response::IntoResponse,
    Extension,
};

use crate::{
    api::{
        middleware::auth::CurrentUser,
        state::AppState,
    },
    web::templates::{HtmlTemplate, UserInfo},
};
use super::is_admin;

#[derive(Template)]
#[template(path = "portal/payments.html")]
pub struct PaymentsTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
}

pub async fn payments_page(
    State(_state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {
    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let template = PaymentsTemplate {
        current_user: Some(user_info),
        is_admin: is_admin(&current_user.member),
    };

    HtmlTemplate(template)
}

// API endpoint for full payments list
pub async fn payments_list_api(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {
    let payments = state.service_context.payment_repo
        .find_by_member(current_user.member.id)
        .await
        .unwrap_or_default();

    if payments.is_empty() {
        return axum::response::Html(
            r#"<div class="p-6 text-center text-gray-500">
                No payment history
            </div>"#.to_string()
        );
    }

    let mut html = String::from(r#"<div class="divide-y">"#);

    for payment in payments {
        let status_badge = match format!("{:?}", payment.status).as_str() {
            "Completed" => r#"<span class="px-2 py-1 text-xs font-medium rounded bg-green-100 text-green-800">Completed</span>"#,
            "Pending" => r#"<span class="px-2 py-1 text-xs font-medium rounded bg-yellow-100 text-yellow-800">Pending</span>"#,
            "Failed" => r#"<span class="px-2 py-1 text-xs font-medium rounded bg-red-100 text-red-800">Failed</span>"#,
            "Refunded" => r#"<span class="px-2 py-1 text-xs font-medium rounded bg-gray-100 text-gray-800">Refunded</span>"#,
            _ => "",
        };

        let description = if payment.description.is_empty() {
            "Membership dues".to_string()
        } else {
            payment.description.clone()
        };

        html.push_str(&format!(
            r#"<div class="px-6 py-4 flex justify-between items-center">
                <div>
                    <p class="font-medium text-gray-900">{}</p>
                    <p class="text-sm text-gray-500">{}</p>
                </div>
                <div class="text-right">
                    <p class="font-medium text-gray-900">${:.2}</p>
                    <div class="mt-1">{}</div>
                </div>
            </div>"#,
            description,
            payment.created_at.format("%B %d, %Y"),
            payment.amount_cents as f64 / 100.0,
            status_badge
        ));
    }

    html.push_str("</div>");
    axum::response::Html(html)
}

// API endpoint for payments summary
pub async fn payments_summary_api(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {
    use crate::domain::PaymentStatus;

    let payments = state.service_context.payment_repo
        .find_by_member(current_user.member.id)
        .await
        .unwrap_or_default();

    let total: i64 = payments.iter()
        .filter(|p| p.status == PaymentStatus::Completed)
        .map(|p| p.amount_cents)
        .sum();

    axum::response::Html(format!("${:.2}", total as f64 / 100.0))
}

// API endpoint for dues status
pub async fn dues_status_api(
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {
    let status = if let Some(dues_until) = current_user.member.dues_paid_until {
        if dues_until > chrono::Utc::now() {
            r#"<span class="text-green-600">Current</span>"#
        } else {
            r#"<span class="text-red-600">Expired</span>"#
        }
    } else {
        r#"<span class="text-yellow-600">Unpaid</span>"#
    };

    axum::response::Html(status.to_string())
}

// API endpoint for next due date
pub async fn next_due_api(
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {
    let next_due = if let Some(dues_until) = current_user.member.dues_paid_until {
        dues_until.format("%B %d, %Y").to_string()
    } else {
        "—".to_string()
    };

    axum::response::Html(next_due)
}
