use axum::{
    extract::State,
    http::{header, StatusCode, HeaderMap},
    Json,
    response::{IntoResponse, Response, Redirect},
};
use axum_extra::extract::CookieJar;
use serde::Serialize;
use serde_json::json;
use crate::api::state::AppState;

#[derive(Serialize)]
pub struct ApiInfo {
    pub name: String,
    pub version: String,
    pub description: String,
    pub status: String,
}

/// Root endpoint with content negotiation:
/// - Browsers (Accept: text/html): redirect to dashboard if logged in, else to login
/// - API clients (Accept: application/json) get API info JSON
pub async fn root(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
) -> Response {
    // Check if the client prefers HTML (browser)
    let accepts_html = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("text/html"))
        .unwrap_or(false);

    if accepts_html {
        // Check if user has a valid session
        if let Some(session_cookie) = jar.get("session") {
            if let Ok(Some(_session)) = state.service_context.auth_service
                .validate_session(session_cookie.value())
                .await
            {
                return Redirect::to("/portal/dashboard").into_response();
            }
        }
        Redirect::to("/login").into_response()
    } else {
        Json(json!({
            "name": "Coterie API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Member management system for clubs and organizations",
            "status": "operational",
            "endpoints": {
                "health": "GET /health - Health check",
                "api_info": "GET /api - API information",
                "public": {
                    "signup": "POST /public/signup - Register new member",
                    "events": "GET /public/events - List public events",
                    "announcements": "GET /public/announcements - List public announcements",
                    "rss": "GET /public/feed/rss - RSS feed",
                    "calendar": "GET /public/feed/calendar - iCal feed"
                },
                "auth": {
                    "login": "POST /api/auth/login - Authenticate",
                    "logout": "POST /api/auth/logout - End session"
                },
                "members": "GET/POST /api/members - Member management (authenticated)",
                "events": "GET/POST /api/events - Event management (authenticated)",
                "payments": "GET/POST /api/payments - Payment management (authenticated)"
            },
            "portal": {
                "login": "/login - Web login page",
                "dashboard": "/portal/dashboard - Member dashboard",
                "admin": "/portal/admin/members - Admin interface"
            },
            "documentation": "https://github.com/IndustriousKraken/coterie"
        })).into_response()
    }
}

pub async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({
        "status": "healthy",
        "timestamp": chrono::Utc::now().to_rfc3339()
    })))
}

pub async fn api_info() -> impl IntoResponse {
    Json(ApiInfo {
        name: "Coterie API".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        description: "Member management system for clubs and organizations".to_string(),
        status: "operational".to_string(),
    })
}