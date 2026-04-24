pub mod templates;
pub mod portal;
pub mod uploads;

use axum::{
    Router,
    routing::{get, post},
};
use crate::api::state::AppState;

/// Escape HTML special characters to prevent XSS in raw HTML responses.
/// Use this for any user-supplied or database-sourced value interpolated
/// into `format!()` HTML strings (HTMX fragment responses, error messages, etc.).
pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(c),
        }
    }
    out
}

pub fn create_web_routes(state: AppState) -> Router {
    Router::new()
        // Setup page (first-run)
        .route("/setup", get(templates::setup::setup_page))
        .route("/setup", post(templates::setup::setup_handler))

        // Auth pages (web interface)
        .route("/login", get(templates::auth::login_page))
        .route("/login", post(templates::auth::login_handler))
        .route("/logout", post(templates::auth::logout_handler))

        // Portal routes
        .nest("/portal", portal::create_portal_routes(state.clone()))

        // Serve uploaded files (with auth check for private content)
        .route("/uploads/:filename", get(uploads::serve_upload))

        .with_state(state)
}