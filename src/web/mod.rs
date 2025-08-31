pub mod templates;
pub mod portal;

use axum::{
    Router,
    routing::{get, post},
};
use crate::api::state::AppState;

pub fn create_web_routes(state: AppState) -> Router {
    Router::new()
        // Auth pages (web interface)
        .route("/login", get(templates::auth::login_page))
        .route("/login", post(templates::auth::login_handler))
        .route("/logout", post(templates::auth::logout_handler))
        
        // Portal routes
        .nest("/portal", portal::create_portal_routes(state.clone()))
        
        .with_state(state)
}