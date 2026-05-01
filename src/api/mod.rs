pub mod docs;
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
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use crate::{
    config::Settings,
    payments::{StripeClient, WebhookDispatcher},
    service::{billing_service::BillingService, ServiceContext},
};
use state::AppState;

pub fn create_app(
    service_context: Arc<ServiceContext>,
    stripe_client: Option<Arc<StripeClient>>,
    webhook_dispatcher: Option<Arc<WebhookDispatcher>>,
    billing_service: Arc<BillingService>,
    settings: Arc<Settings>,
) -> Router {
    let cors_layer = build_cors_layer(&settings);
    let app_state = AppState::new(
        service_context,
        stripe_client,
        webhook_dispatcher,
        billing_service,
        settings,
    );

    Router::new()
        // Root and health endpoints
        .route("/", get(handlers::root::root))
        .route("/health", get(handlers::root::health_check))
        .route("/api", get(handlers::root::api_info))

        // OpenAPI / Swagger UI for the public API. The UI is served at
        // /api/docs and the raw spec at /api/docs/openapi.json so frontend
        // developers can either browse interactively or codegen a client.
        .merge(SwaggerUi::new("/api/docs")
            .url("/api/docs/openapi.json", docs::ApiDoc::openapi()))

        // Auth routes
        .route("/auth/login", post(handlers::auth::login))
        .route("/auth/logout", post(handlers::auth::logout))

        // API routes — narrowly scoped to:
        //   1. The Stripe webhook (Stripe POSTs here).
        //   2. The saved-card endpoints the portal frontend calls
        //      directly via `fetch()` (see payment_methods.html).
        // Everything that used to live here (admin CRUD on members /
        // events / announcements, JSON manual-payment / waive, the
        // entire /admin/* mount) was deleted in favour of the portal
        // admin pages, which are the single source of truth for admin
        // actions and carry the audit-log + integration-event
        // side-effects the JSON wrappers were missing.
        .nest("/api", api_routes(app_state.clone()))

        // Public routes (for website integration)
        .nest("/public", public_routes(app_state.clone()))

        // Add state to the router
        .with_state(app_state.clone())

        // Middleware. Order matters: `.layer()` calls wrap from the
        // inside out, so the LAST `.layer(...)` is OUTERMOST and runs
        // FIRST on incoming requests.
        //
        // CSRF protection is NOT layered here. It's applied once at the
        // top of the merged app in `main.rs` so it covers BOTH the API
        // surface and the portal/web routes that get added via
        // `Router::merge`. Layers added before a `merge` call do not
        // propagate to the merged routes in axum 0.7 — applying CSRF
        // here would leave every state-changing /portal/* route
        // unprotected.
        .layer(axum::middleware::from_fn_with_state(
            app_state.clone(),
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
    Router::new().nest("/payments", payment_routes(state.clone()))
}

fn payment_routes(state: AppState) -> Router<AppState> {
    Router::new()
        // Public webhook endpoint (no auth)
        .route("/webhook/stripe", post(handlers::payments::stripe_webhook))
        // Saved-card management. The portal frontend POSTs directly
        // to these (`fetch('/api/payments/cards/...')` in
        // payment_methods.html) because Stripe.js needs a JSON
        // surface, not an HTMX one. CSRF is enforced at the
        // application root by `csrf_protect_unless_exempt`; only the
        // auth gate is layered here. The portal's fetch() calls stamp
        // the X-CSRF-Token header from `<meta name="csrf-token">`.
        .nest("/", Router::new()
            .route("/cards", get(handlers::payments::list_saved_cards))
            .route("/cards", post(handlers::payments::save_card))
            .route("/cards/setup-intent", post(handlers::payments::create_setup_intent))
            .route("/cards/:card_id", delete(handlers::payments::delete_saved_card))
            .route("/cards/:card_id/default", put(handlers::payments::set_default_card))
            .route_layer(axum::middleware::from_fn_with_state(
                state.clone(),
                middleware::auth::require_auth,
            ))
        )
}

fn public_routes(_state: AppState) -> Router<AppState> {
    Router::new()
        .route("/signup", post(handlers::public::signup))
        .route("/donate", post(handlers::public::donate))
        .route("/events", get(handlers::public::list_events))
        .route("/events/private-count", get(handlers::public::private_event_count))
        .route("/announcements", get(handlers::public::list_announcements))
        .route("/announcements/private-count", get(handlers::announcements::private_count))
        .route("/feed/rss", get(handlers::public::rss_feed))
        .route("/feed/calendar", get(handlers::public::calendar_feed))
}

