use askama::Template;
use axum::{
    extract::State,
    response::{IntoResponse, Redirect, Response},
    Extension,
};

use crate::{
    api::{middleware::auth::CurrentUser, state::AppState},
    web::templates::{HtmlTemplate, UserInfo},
};
use super::is_admin;

#[derive(Template)]
#[template(path = "portal/restore.html")]
pub struct RestoreTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    /// Pre-formatted date the membership expired, if known.
    pub expired_on: Option<String>,
}

/// Landing page for Expired members. Active/Honorary members who somehow
/// navigate here are redirected to the dashboard — they don't belong on
/// the "restore your account" page.
pub async fn restore_page(
    State(_state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
) -> Response {
    use crate::domain::MemberStatus;

    if !matches!(current_user.member.status, MemberStatus::Expired) {
        return Redirect::to("/portal/dashboard").into_response();
    }

    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let expired_on = current_user.member.dues_paid_until
        .map(|d| d.format("%B %d, %Y").to_string());

    let template = RestoreTemplate {
        current_user: Some(user_info),
        is_admin: is_admin(&current_user.member),
        expired_on,
    };

    HtmlTemplate(template).into_response()
}
