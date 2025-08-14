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

pub async fn create() -> Result<StatusCode> {
    Ok(StatusCode::NOT_IMPLEMENTED)
}

pub async fn get(Path(_id): Path<Uuid>) -> Result<StatusCode> {
    Ok(StatusCode::NOT_IMPLEMENTED)
}

pub async fn list_by_member(Path(_member_id): Path<Uuid>) -> Result<StatusCode> {
    Ok(StatusCode::NOT_IMPLEMENTED)
}

pub async fn stripe_webhook() -> Result<StatusCode> {
    Ok(StatusCode::NOT_IMPLEMENTED)
}