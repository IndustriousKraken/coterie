pub mod handlers;
pub mod middleware;
pub mod state;

use axum::{
    Router,
    http::{header, Method},
    routing::{get, post, put, delete},
};
use tower_http::{
    compression::CompressionLayer,
    cors::{CorsLayer, AllowOrigin},
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
    let cors_layer = build_cors_layer(&settings);
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
        .with_state(app_state.clone())

        // Middleware
        .layer(axum::middleware::from_fn_with_state(
            app_state,
            middleware::security_headers::security_headers,
        ))
        .layer(CompressionLayer::new())
        .layer(cors_layer)
        .layer(TraceLayer::new_for_http())
}

/// Build CORS layer from configuration. If `cors_origins` is set, only those
/// origins are allowed. Otherwise the layer is restrictive (same-origin only).
fn build_cors_layer(settings: &Settings) -> CorsLayer {
    let origins: Vec<_> = settings.server.cors_origins
        .as_deref()
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect();

    let layer = if origins.is_empty() {
        // No configured origins → same-origin only (no Access-Control-Allow-Origin).
        CorsLayer::new()
    } else {
        CorsLayer::new().allow_origin(AllowOrigin::list(origins))
    };

    layer
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE, Method::OPTIONS])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION, "X-CSRF-Token".parse().unwrap()])
        .allow_credentials(true)
}

fn api_routes(state: AppState) -> Router<AppState> {
    Router::new()
        .nest("/members", member_routes(state.clone()))
        .nest("/events", event_routes_with_auth(state.clone()))
        .nest("/announcements", announcement_routes_with_auth(state.clone()))
        .nest("/payments", payment_routes(state.clone()))
}

fn member_routes(state: AppState) -> Router<AppState> {
    // Admin-only operations: listing everyone, creating, mutating, activating, expiring.
    let admin_only = Router::new()
        .route("/", get(handlers::members::list))
        .route("/", post(handlers::members::create))
        .route("/:id", put(handlers::members::update))
        .route("/:id", delete(handlers::members::delete))
        .route("/:id/activate", post(handlers::members::activate))
        .route("/:id/expire", post(handlers::members::expire))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            middleware::auth::require_admin,
        ));

    // Authenticated-but-not-admin: reading a single member record. The handler
    // performs the self-or-admin ownership check (see handlers::members::get).
    let authed = Router::new()
        .route("/:id", get(handlers::members::get))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            middleware::auth::require_auth,
        ));

    Router::new().merge(admin_only).merge(authed)
}

fn event_routes_with_auth(state: AppState) -> Router<AppState> {
    // Admin-only: creating/updating/deleting events.
    let admin_only = Router::new()
        .route("/", post(handlers::events::create))
        .route("/:id", put(handlers::events::update))
        .route("/:id", delete(handlers::events::delete))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            middleware::auth::require_admin,
        ));

    // Any authenticated member can RSVP.
    let member_only = Router::new()
        .route("/:id/register", post(handlers::events::register))
        .route("/:id/cancel", post(handlers::events::cancel))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            middleware::auth::require_auth,
        ));

    Router::new()
        // Public reads (visibility is enforced inside the handlers).
        .route("/", get(handlers::events::list))
        .route("/:id", get(handlers::events::get))
        .merge(admin_only)
        .merge(member_only)
}

fn announcement_routes_with_auth(state: AppState) -> Router<AppState> {
    // Admin-only mutations.
    let admin_only = Router::new()
        .route("/", post(handlers::announcements::create))
        .route("/:id", put(handlers::announcements::update))
        .route("/:id", delete(handlers::announcements::delete))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            middleware::auth::require_admin,
        ));

    Router::new()
        // Public reads (visibility enforced inside the handlers).
        .route("/", get(handlers::announcements::list))
        .route("/:id", get(handlers::announcements::get))
        .route("/private-count", get(handlers::announcements::private_count))
        .merge(admin_only)
}

fn payment_routes(state: AppState) -> Router<AppState> {
    Router::new()
        // Public webhook endpoint (no auth)
        .route("/webhook/stripe", post(handlers::payments::stripe_webhook))
        // Protected payment endpoints. CSRF middleware is layered
        // INSIDE require_auth so it can read SessionInfo populated by
        // the auth pass. The portal's fetch() calls all stamp the
        // X-CSRF-Token header (see templates/portal/payment_methods.html);
        // require_csrf is a defense-in-depth backstop in case CORS
        // policy widens or an XSS slips through and tries to abuse
        // these card-management endpoints.
        .nest("/", Router::new()
            .route("/", post(handlers::payments::create))
            .route("/:id", get(handlers::payments::get))
            .route("/member/:member_id", get(handlers::payments::list_by_member))
            // Saved card (payment method) routes
            .route("/cards", get(handlers::payments::list_saved_cards))
            .route("/cards", post(handlers::payments::save_card))
            .route("/cards/setup-intent", post(handlers::payments::create_setup_intent))
            .route("/cards/:card_id", delete(handlers::payments::delete_saved_card))
            .route("/cards/:card_id/default", put(handlers::payments::set_default_card))
            .route_layer(axum::middleware::from_fn_with_state(
                state.clone(),
                middleware::auth::require_csrf,
            ))
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
        .route("/events/private-count", get(handlers::public::private_event_count))
        .route("/announcements", get(handlers::public::list_announcements))
        .route("/announcements/private-count", get(handlers::announcements::private_count))
        .route("/feed/rss", get(handlers::public::rss_feed))
        .route("/feed/calendar", get(handlers::public::calendar_feed))
}

fn admin_routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/audit-log", get(handlers::admin::audit_log))
        .route("/expired-check", post(handlers::admin::check_expired))
        // Admin-only payment operations
        .route("/payments/manual", post(handlers::payments::create_manual))
        .route("/payments/waive", post(handlers::payments::waive))
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