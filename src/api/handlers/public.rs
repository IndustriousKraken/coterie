use axum::{
    extract::State,
    http::StatusCode,
    Json,
};

use crate::{
    api::state::AppState,
    error::Result,
};

pub async fn signup() -> Result<StatusCode> {
    Ok(StatusCode::NOT_IMPLEMENTED)
}

pub async fn list_events(State(_state): State<AppState>) -> Result<Json<Vec<String>>> {
    Ok(Json(vec![]))
}

pub async fn list_announcements(State(_state): State<AppState>) -> Result<Json<Vec<String>>> {
    Ok(Json(vec![]))
}

pub async fn rss_feed() -> Result<StatusCode> {
    Ok(StatusCode::NOT_IMPLEMENTED)
}

pub async fn calendar_feed() -> Result<StatusCode> {
    Ok(StatusCode::NOT_IMPLEMENTED)
}