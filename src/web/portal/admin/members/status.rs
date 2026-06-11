use std::sync::Arc;

use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Extension,
};

use crate::{
    api::middleware::auth::CurrentUser, service::member_service::MemberService,
    web::portal::admin::partials,
};

pub async fn admin_activate_member(
    State(member_service): State<Arc<MemberService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
) -> impl IntoResponse {
    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return partials::member_row_error("Invalid member ID"),
    };

    match member_service.activate(current_user.member.id, id).await {
        Ok(member) => {
            let mt_name = member_service.membership_type_name(&member).await;
            partials::member_row_flash(&member, mt_name, "active")
        }
        Err(e) => partials::member_row_error(&format!("Error: {}", e)),
    }
}

pub async fn admin_suspend_member(
    State(member_service): State<Arc<MemberService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
) -> impl IntoResponse {
    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return partials::member_row_error("Invalid member ID"),
    };

    match member_service.suspend(current_user.member.id, id).await {
        Ok(member) => {
            let mt_name = member_service.membership_type_name(&member).await;
            partials::member_row_flash(&member, mt_name, "suspended")
        }
        Err(e) => partials::member_row_error(&format!("Error: {}", e)),
    }
}

pub async fn admin_expire_now(
    State(member_service): State<Arc<MemberService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
) -> impl IntoResponse {
    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return partials::admin_alert("error", "Invalid member ID", false),
    };

    match member_service.expire_now(current_user.member.id, id).await {
        Ok(_) => partials::admin_alert("warning", "Member dues have been expired.", true),
        Err(e) => partials::admin_alert("error", &format!("Error: {}", e), false),
    }
}
