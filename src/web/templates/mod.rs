pub mod auth;
pub mod reset;
pub mod setup;
pub mod verify;

use askama::Template;
use axum::{
    response::{Html, IntoResponse, Response},
    http::StatusCode,
};

use crate::api::{
    middleware::auth::{CurrentUser, SessionInfo},
    state::AppState,
};

/// Context every page that extends `layouts/base.html` carries.
///
/// Embedding this struct as `pub base: BaseContext` on each template
/// struct keeps the layout-level fields in one place: adding a new
/// global template variable means updating exactly this struct and
/// the layout, not 40 individual page structs.
///
/// `csrf_token` lives here because the layout renders a global
/// `<meta name="csrf-token">` that the HTMX `htmx:configRequest`
/// handler reads to stamp `X-CSRF-Token` on every state-changing
/// request — including the global logout button. Per-page templates
/// that omit the token would silently break logout from that page.
#[derive(Debug, Clone, Default)]
pub struct BaseContext {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub csrf_token: String,
}

impl BaseContext {
    /// Build a base context for an authenticated portal page. Mints a
    /// fresh CSRF token bound to the active session — every authenticated
    /// page renders with a usable token, so HTMX state-changing actions
    /// (including the global logout button in the layout) work from any
    /// page.
    pub async fn for_member(
        state: &AppState,
        current_user: &CurrentUser,
        session: &SessionInfo,
    ) -> Self {
        let csrf_token = state
            .service_context
            .csrf_service
            .generate_token(&session.session_id)
            .await
            .unwrap_or_default();
        Self {
            current_user: Some(UserInfo {
                id: current_user.member.id.to_string(),
                username: current_user.member.username.clone(),
                email: current_user.member.email.clone(),
            }),
            is_admin: current_user.member.is_admin,
            csrf_token,
        }
    }

    /// Pre-auth pages (login, setup, password reset). No session, no
    /// CSRF binding — the layout still renders the meta tag so HTMX
    /// won't crash, but the token is empty. Forms on these pages POST
    /// to CSRF-exempt endpoints (login, signup) or supply tokens
    /// out-of-band (password reset link).
    pub fn for_anon() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone)]
pub struct UserInfo {
    pub id: String,
    pub username: String,
    pub email: String,
}

// Make askama templates work with axum
pub struct HtmlTemplate<T>(pub T);

impl<T> IntoResponse for HtmlTemplate<T>
where
    T: Template,
{
    fn into_response(self) -> Response {
        match self.0.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            ).into_response(),
        }
    }
}