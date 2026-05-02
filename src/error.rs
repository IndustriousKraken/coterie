use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, AppError>;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Database error: {0}")]
    Database(String),
    
    #[error("Not found: {0}")]
    NotFound(String),
    
    #[error("Unauthorized")]
    Unauthorized,
    
    #[error("Forbidden")]
    Forbidden,
    
    #[error("Bad request: {0}")]
    BadRequest(String),
    
    #[error("Conflict: {0}")]
    Conflict(String),
    
    #[error("Internal server error: {0}")]
    Internal(String),

    #[error("Integration error: {0}")]
    Integration(String),
    
    #[error("Validation error: {0}")]
    Validation(String),
    
    #[error("Service unavailable: {0}")]
    ServiceUnavailable(String),
    
    #[error("External service error: {0}")]
    External(String),

    #[error("Too many requests")]
    TooManyRequests,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            AppError::Database(ref msg) => {
                tracing::error!("Database error: {}", msg);
                (StatusCode::INTERNAL_SERVER_ERROR, "Database error occurred")
            }
            AppError::NotFound(ref msg) => (StatusCode::NOT_FOUND, msg.as_str()),
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "Unauthorized"),
            AppError::Forbidden => (StatusCode::FORBIDDEN, "Forbidden"),
            AppError::BadRequest(ref msg) => (StatusCode::BAD_REQUEST, msg.as_str()),
            AppError::Conflict(ref msg) => (StatusCode::CONFLICT, msg.as_str()),
            AppError::Internal(ref msg) => {
                tracing::error!("Internal error: {}", msg);
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
            }
            AppError::Integration(ref msg) => {
                tracing::error!("Integration error: {}", msg);
                (StatusCode::SERVICE_UNAVAILABLE, msg.as_str())
            }
            AppError::Validation(ref msg) => (StatusCode::UNPROCESSABLE_ENTITY, msg.as_str()),
            AppError::ServiceUnavailable(ref msg) => (StatusCode::SERVICE_UNAVAILABLE, msg.as_str()),
            AppError::External(ref msg) => {
                // Inner message often contains raw vendor strings (Stripe
                // request IDs, "No such customer: cus_…", Discord HTTP
                // bodies). Log the detail for ops, but return a generic
                // string to the client — there's nothing actionable for
                // an end user in the upstream's error text.
                tracing::error!("External service error: {}", msg);
                (
                    StatusCode::BAD_GATEWAY,
                    "Upstream service error. Please try again or contact support.",
                )
            }
            AppError::TooManyRequests => (StatusCode::TOO_MANY_REQUESTS, "Too many requests. Please try again later.")
        };

        let body = Json(json!({
            "error": error_message,
        }));

        (status, body).into_response()
    }
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        AppError::Database(err.to_string())
    }
}