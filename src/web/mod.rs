pub mod templates;
pub mod portal;
pub mod uploads;

use axum::{
    Router,
    routing::{get, post},
};
use tower_http::services::ServeDir;
use crate::api::state::AppState;

pub fn create_web_routes(state: AppState) -> Router {
    // Get uploads directory from settings
    let uploads_dir = state.settings.server.uploads_dir.clone();

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

        // Serve uploaded files
        .nest_service("/uploads", ServeDir::new(&uploads_dir))

        .with_state(state)
}