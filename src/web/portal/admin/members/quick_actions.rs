//! Quick Actions card handlers: send a password reset on the member's
//! behalf, and delete a payment-free member. Thin wrappers — the
//! guards, audit, and side effects live in `MemberService`
//! (see the admin-members capability spec).

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

pub async fn admin_send_password_reset(
    State(member_service): State<Arc<MemberService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
) -> axum::response::Response {
    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return partials::admin_alert("error", "Invalid member ID", false).into_response(),
    };

    match member_service
        .send_password_reset(current_user.member.id, id)
        .await
    {
        Ok(()) => partials::admin_alert("success", "Password reset email sent.", false).into_response(),
        Err(e) => partials::admin_alert("error", &format!("Reset failed: {}", e), false).into_response(),
    }
}

pub async fn admin_delete_member(
    State(member_service): State<Arc<MemberService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
) -> axum::response::Response {
    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return partials::admin_alert("error", "Invalid member ID", false).into_response(),
    };

    match member_service.delete(current_user.member.id, id).await {
        // HX-Redirect: the member page no longer exists — navigate the
        // admin back to the list instead of swapping a fragment into it.
        Ok(()) => ([("HX-Redirect", "/portal/admin/members")], "deleted").into_response(),
        Err(e) => partials::admin_alert("error", &format!("Delete failed: {}", e), false).into_response(),
    }
}
