use axum::{
    extract::Path,
    http::StatusCode,
};
use uuid::Uuid;

use crate::error::Result;

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