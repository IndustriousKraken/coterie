use axum::{
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};

use crate::api::state::AppState;

/// Middleware that checks if initial setup is needed.
/// If no admin user exists, redirects to /setup.
/// Allows access to /setup, /login, and static assets without redirect.
pub async fn require_setup(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path();

    // Allow these paths without checking for admin
    if path.starts_with("/setup")
        || path.starts_with("/static")
        || path.starts_with("/assets")
        || path.starts_with("/favicon")
    {
        return next.run(request).await;
    }

    // Check if any admin exists
    let has_admin = check_admin_exists(&state).await;

    if !has_admin {
        // Redirect to setup page
        return Redirect::to("/setup").into_response();
    }

    next.run(request).await
}

/// Check if at least one admin user exists in the database
async fn check_admin_exists(state: &AppState) -> bool {
    // Query for any member with "ADMIN" in their notes
    let result: Result<Option<(i64,)>, _> = sqlx::query_as(
        r#"
        SELECT 1 as exists_flag
        FROM members
        WHERE notes LIKE '%ADMIN%'
        LIMIT 1
        "#,
    )
    .fetch_optional(&state.service_context.db_pool)
    .await;

    match result {
        Ok(Some(_)) => true,
        Ok(None) => false,
        Err(e) => {
            tracing::error!("Failed to check for admin: {}", e);
            // On error, assume setup is not needed to avoid blocking
            true
        }
    }
}
