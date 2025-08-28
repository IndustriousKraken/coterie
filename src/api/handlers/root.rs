use axum::{http::StatusCode, Json, response::IntoResponse};
use serde::Serialize;
use serde_json::json;

#[derive(Serialize)]
pub struct ApiInfo {
    pub name: String,
    pub version: String,
    pub description: String,
    pub status: String,
}

pub async fn root() -> impl IntoResponse {
    Json(json!({
        "name": "Coterie API",
        "version": env!("CARGO_PKG_VERSION"),
        "description": "Member management system for clubs and organizations",
        "status": "operational",
        "endpoints": {
            "health": "/health",
            "api": "/api",
            "auth": "/auth/login",
            "public": "/public",
            "admin": "/admin"
        },
        "documentation": "https://github.com/yourusername/coterie"
    }))
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