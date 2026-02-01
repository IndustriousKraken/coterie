pub mod handlers;
pub mod middleware;
pub mod state;

use axum::{
    Router,
    routing::{get, post, put, delete},
};
use tower_http::{
    compression::CompressionLayer,
    cors::CorsLayer,
    trace::TraceLayer,
};
use std::sync::Arc;

use crate::{
    config::Settings,
    payments::StripeClient,
    service::ServiceContext,
};
use state::AppState;

pub fn create_app(
    service_context: Arc<ServiceContext>,
    stripe_client: Option<Arc<StripeClient>>,
    settings: Arc<Settings>,
) -> Router {
    let app_state = AppState::new(service_context, stripe_client, settings);

    Router::new()
        // Root and health endpoints
        .route("/", get(handlers::root::root))
        .route("/health", get(handlers::root::health_check))
        .route("/api", get(handlers::root::api_info))
        
        // Auth routes
        .route("/auth/login", post(handlers::auth::login))
        .route("/auth/logout", post(handlers::auth::logout))
        
        // API routes
        .nest("/api", api_routes(app_state.clone()))
        
        // Public routes (for website integration)
        .nest("/public", public_routes(app_state.clone()))
        
        // Admin routes
        .nest("/admin", admin_routes(app_state.clone()))
        
        // Add state to the router
        .with_state(app_state)
        
        // Middleware
        .layer(CompressionLayer::new())
        .layer(CorsLayer::permissive()) // Configure properly for production
        .layer(TraceLayer::new_for_http())
}

fn api_routes(state: AppState) -> Router<AppState> {
    Router::new()
        .nest("/members", member_routes(state.clone()))
        .nest("/events", event_routes_with_auth(state.clone()))
        .nest("/announcements", announcement_routes_with_auth(state.clone()))
        .nest("/payments", payment_routes(state.clone()))
}

fn member_routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/", get(handlers::members::list))
        .route("/", post(handlers::members::create))
        .route("/:id", get(handlers::members::get))
        .route("/:id", put(handlers::members::update))
        .route("/:id", delete(handlers::members::delete))
        .route("/:id/activate", post(handlers::members::activate))
        .route("/:id/expire", post(handlers::members::expire))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            middleware::auth::require_auth,
        ))
}

fn event_routes_with_auth(state: AppState) -> Router<AppState> {
    Router::new()
        // Public routes (no auth required for viewing)
        .route("/", get(handlers::events::list))
        .route("/:id", get(handlers::events::get))
        // Protected routes - wrapped in a nested router with auth middleware
        .nest("/", Router::new()
            .route("/", post(handlers::events::create))
            .route("/:id", put(handlers::events::update))
            .route("/:id", delete(handlers::events::delete))
            .route("/:id/register", post(handlers::events::register))
            .route("/:id/cancel", post(handlers::events::cancel))
            .route_layer(axum::middleware::from_fn_with_state(
                state.clone(),
                middleware::auth::require_auth,
            ))
        )
}

fn announcement_routes_with_auth(state: AppState) -> Router<AppState> {
    Router::new()
        // Public routes (no auth required for viewing public announcements)
        .route("/", get(handlers::announcements::list))
        .route("/:id", get(handlers::announcements::get))
        .route("/private-count", get(handlers::announcements::private_count))
        // Protected routes - require auth
        .nest("/", Router::new()
            .route("/", post(handlers::announcements::create))
            .route("/:id", put(handlers::announcements::update))
            .route("/:id", delete(handlers::announcements::delete))
            .route_layer(axum::middleware::from_fn_with_state(
                state.clone(),
                middleware::auth::require_auth,
            ))
        )
}

fn payment_routes(state: AppState) -> Router<AppState> {
    Router::new()
        // Public webhook endpoint (no auth)
        .route("/webhook/stripe", post(handlers::payments::stripe_webhook))
        // Protected payment endpoints
        .nest("/", Router::new()
            .route("/", post(handlers::payments::create))
            .route("/:id", get(handlers::payments::get))
            .route("/member/:member_id", get(handlers::payments::list_by_member))
            .route("/manual", post(handlers::payments::create_manual))
            .route("/waive", post(handlers::payments::waive))
            .route_layer(axum::middleware::from_fn_with_state(
                state.clone(),
                middleware::auth::require_auth,
            ))
        )
}

fn public_routes(_state: AppState) -> Router<AppState> {
    Router::new()
        .route("/signup", post(handlers::public::signup))
        .route("/events", get(handlers::public::list_events))
        .route("/announcements", get(handlers::public::list_announcements))
        .route("/announcements/private-count", get(handlers::announcements::private_count))
        .route("/feed/rss", get(handlers::public::rss_feed))
        .route("/feed/calendar", get(handlers::public::calendar_feed))
}

fn admin_routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/stats", get(handlers::admin::stats))
        .route("/audit-log", get(handlers::admin::audit_log))
        .route("/expired-check", post(handlers::admin::check_expired))
        // Settings management routes
        .route("/settings", get(handlers::settings::list_settings))
        .route("/settings/batch", put(handlers::settings::batch_update))
        .route("/settings/category/:category", get(handlers::settings::get_category))
        .route("/settings/payment-config", get(handlers::settings::get_payment_config))
        .route("/settings/membership-config", get(handlers::settings::get_membership_config))
        .route("/settings/:key", get(handlers::settings::get_setting))
        .route("/settings/:key", put(handlers::settings::update_setting))
        // Configurable types management routes
        .route("/types", get(handlers::types::get_all_types))
        .nest("/types/events", event_type_routes())
        .nest("/types/announcements", announcement_type_routes())
        .nest("/types/memberships", membership_type_routes())
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            middleware::auth::require_admin,
        ))
        .with_state(state)
}

fn event_type_routes() -> Router<AppState> {
    Router::new()
        .route("/", get(handlers::types::list_event_types))
        .route("/", post(handlers::types::create_event_type))
        .route("/reorder", post(handlers::types::reorder_event_types))
        .route("/seed", post(handlers::types::seed_event_types))
        .route("/slug/:slug", get(handlers::types::get_event_type_by_slug))
        .route("/:id", get(handlers::types::get_event_type))
        .route("/:id", put(handlers::types::update_event_type))
        .route("/:id", delete(handlers::types::delete_event_type))
}

fn announcement_type_routes() -> Router<AppState> {
    Router::new()
        .route("/", get(handlers::types::list_announcement_types))
        .route("/", post(handlers::types::create_announcement_type))
        .route("/reorder", post(handlers::types::reorder_announcement_types))
        .route("/seed", post(handlers::types::seed_announcement_types))
        .route("/slug/:slug", get(handlers::types::get_announcement_type_by_slug))
        .route("/:id", get(handlers::types::get_announcement_type))
        .route("/:id", put(handlers::types::update_announcement_type))
        .route("/:id", delete(handlers::types::delete_announcement_type))
}

fn membership_type_routes() -> Router<AppState> {
    Router::new()
        .route("/", get(handlers::types::list_membership_types))
        .route("/", post(handlers::types::create_membership_type))
        .route("/reorder", post(handlers::types::reorder_membership_types))
        .route("/seed", post(handlers::types::seed_membership_types))
        .route("/pricing", get(handlers::types::list_all_pricing))
        .route("/slug/:slug", get(handlers::types::get_membership_type_by_slug))
        .route("/:id", get(handlers::types::get_membership_type))
        .route("/:id", put(handlers::types::update_membership_type))
        .route("/:id", delete(handlers::types::delete_membership_type))
        .route("/:id/pricing", get(handlers::types::get_membership_pricing))
}