use askama::Template;
use axum::{
    extract::State,
    response::{IntoResponse, Redirect, Response},
    Extension,
};

use crate::{
    api::{
        middleware::auth::{CurrentUser, SessionInfo},
        state::AppState,
    },
    web::templates::{BaseContext, HtmlTemplate},
};

#[derive(Template)]
#[template(path = "portal/restore.html")]
pub struct RestoreTemplate {
    pub base: BaseContext,
    /// Pre-formatted date the membership expired, if known.
    pub expired_on: Option<String>,
}

/// Landing page for Expired members. Active/Honorary members who somehow
/// navigate here are redirected to the dashboard — they don't belong on
/// the "restore your account" page.
pub async fn restore_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session): Extension<SessionInfo>,
) -> Response {
    use crate::domain::MemberStatus;

    if !matches!(current_user.member.status, MemberStatus::Expired) {
        return Redirect::to("/portal/dashboard").into_response();
    }

    let expired_on = current_user.member.dues_paid_until
        .map(|d| d.format("%B %d, %Y").to_string());

    let template = RestoreTemplate {
        base: BaseContext::for_member(&state, &current_user, &session).await,
        expired_on,
    };

    HtmlTemplate(template).into_response()
}
