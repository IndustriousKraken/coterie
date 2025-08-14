use axum::{
    extract::State,
    http::StatusCode,
    Json,
};

use crate::{
    api::state::AppState,
    error::Result,
};

pub async fn stats(State(_state): State<AppState>) -> Result<Json<String>> {
    Ok(Json("Stats not implemented".to_string()))
}

pub async fn audit_log(State(_state): State<AppState>) -> Result<Json<Vec<String>>> {
    Ok(Json(vec![]))
}

pub async fn check_expired(State(_state): State<AppState>) -> Result<StatusCode> {
    Ok(StatusCode::NOT_IMPLEMENTED)
}