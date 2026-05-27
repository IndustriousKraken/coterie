pub mod docs;
pub mod handlers;
pub mod middleware;
pub mod state;

use axum::{
    Router,
    http::{header, Method},
    routing::{get, post},
};
use tower_http::{
    compression::CompressionLayer,
    cors::{CorsLayer, AllowOrigin},
    trace::TraceLayer,
};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use crate::config::Settings;
use state::AppState;

/// Build the API router on top of a caller-owned [`AppState`].
///
/// The caller (currently `main.rs`) constructs exactly one `AppState`
/// for the process and hands the same value to `create_app` and
/// `create_web_routes`. Sharing the state means the per-IP rate
/// limiters (`login_limiter`, `money_limiter`) and the first-boot
/// `setup_lock` are shared across both surfaces — without this an
/// attacker hitting `/auth/login` on one router and `/login` on the
/// other would get 2× the budget, and two concurrent setup-wizard
/// POSTs (web vs api) could both pass the "no admin yet" check.
pub fn create_app(app_state: AppState) -> Router {
    let cors_layer = build_cors_layer(app_state.settings.as_ref());

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
        .route("/auth/login/totp", post(handlers::auth::login_totp))
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
        // Saved-card Stripe.js entry points. These are the only two
        // endpoints under /api/payments/cards/*: the SetupIntent
        // creation and the post-confirmation record-pm save. The
        // portal frontend `fetch()`-es them directly from
        // payment_methods.html because Stripe.js requires JSON in /
        // JSON out. List, delete, and set-default flows live under
        // /portal/api/payments/cards/* as HTML fragments for HTMX.
        // CSRF is enforced at the application root by
        // `csrf_protect_unless_exempt`; only the auth gate is layered
        // here. The portal's fetch() calls stamp the X-CSRF-Token
        // header from `<meta name="csrf-token">`.
        .nest("/", Router::new()
            .route("/cards", post(handlers::payments::save_card))
            .route("/cards/setup-intent", post(handlers::payments::create_setup_intent))
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

