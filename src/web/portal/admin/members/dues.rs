use std::sync::Arc;

use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Extension,
};
use serde::Deserialize;

use crate::{
    api::middleware::auth::CurrentUser, repository::PaymentRepository,
    service::member_service::MemberService, web::portal::admin::partials,
};

#[derive(Debug, Deserialize)]
pub struct ExtendDuesForm {
    pub months: i32,
    #[allow(dead_code)]
    pub csrf_token: String,
}

#[derive(Debug, Deserialize)]
pub struct SetDuesForm {
    pub dues_until: String,
    #[allow(dead_code)]
    pub csrf_token: String,
}

pub async fn admin_extend_dues(
    State(member_service): State<Arc<MemberService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
    axum::Form(form): axum::Form<ExtendDuesForm>,
) -> impl IntoResponse {
    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return partials::admin_alert("error", "Invalid member ID", false),
    };

    match member_service
        .extend_dues(current_user.member.id, id, form.months)
        .await
    {
        Ok(member) => {
            let new_dues = member
                .dues_paid_until
                .map(|d| d.format("%B %d, %Y").to_string())
                .unwrap_or_else(|| "—".to_string());
            partials::admin_alert(
                "success",
                &format!("Dues extended! New expiration: {}", new_dues),
                true,
            )
        }
        Err(e) => partials::admin_alert("error", &format!("Error: {}", e), false),
    }
}

pub async fn admin_set_dues(
    State(member_service): State<Arc<MemberService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
    axum::Form(form): axum::Form<SetDuesForm>,
) -> impl IntoResponse {
    use chrono::NaiveDate;

    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return partials::admin_alert("error", "Invalid member ID", false),
    };

    let naive_date = match NaiveDate::parse_from_str(&form.dues_until, "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => return partials::admin_alert("error", "Invalid date format", false),
    };

    match member_service
        .set_dues(current_user.member.id, id, naive_date)
        .await
    {
        Ok(member) => {
            let dues = member
                .dues_paid_until
                .map(|d| d.format("%B %d, %Y").to_string())
                .unwrap_or_else(|| "—".to_string());
            partials::admin_alert("success", &format!("Dues date set to: {}", dues), true)
        }
        Err(e) => partials::admin_alert("error", &format!("Error: {}", e), false),
    }
}

pub async fn admin_member_payments(
    State(payment_repo): State<Arc<dyn PaymentRepository>>,
    Extension(_current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
) -> impl IntoResponse {
    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return partials::admin_alert("error", "Invalid member ID", false),
    };

    let payments = payment_repo.find_by_member(id).await.unwrap_or_default();

    let rows = payments
        .iter()
        .map(partials::admin_payment_row_from)
        .collect();
    partials::admin_payment_list(rows)
}
