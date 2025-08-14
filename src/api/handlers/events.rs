use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use uuid::Uuid;

use crate::{
    api::state::AppState,
    error::Result,
};

pub async fn list(State(_state): State<AppState>) -> Result<Json<Vec<String>>> {
    Ok(Json(vec![]))
}

pub async fn get(Path(_id): Path<Uuid>) -> Result<StatusCode> {
    Ok(StatusCode::NOT_IMPLEMENTED)
}

pub async fn create() -> Result<StatusCode> {
    Ok(StatusCode::NOT_IMPLEMENTED)
}

pub async fn update(Path(_id): Path<Uuid>) -> Result<StatusCode> {
    Ok(StatusCode::NOT_IMPLEMENTED)
}

pub async fn delete(Path(_id): Path<Uuid>) -> Result<StatusCode> {
    Ok(StatusCode::NOT_IMPLEMENTED)
}

pub async fn register(Path(_id): Path<Uuid>) -> Result<StatusCode> {
    Ok(StatusCode::NOT_IMPLEMENTED)
}

pub async fn cancel(Path(_id): Path<Uuid>) -> Result<StatusCode> {
    Ok(StatusCode::NOT_IMPLEMENTED)
}