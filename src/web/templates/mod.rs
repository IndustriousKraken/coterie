pub mod auth;

use askama::Template;
use axum::{
    response::{Html, IntoResponse, Response},
    http::StatusCode,
};

// Base template data that all templates will have access to
#[derive(Debug, Clone)]
pub struct BaseContext {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
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