use std::sync::atomic::Ordering;

use axum::{
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};

use crate::api::state::AppState;

/// Middleware that checks if initial setup is needed.
///
/// If no admin user exists, redirects to `/setup`. Allows access to
/// `/setup`, `/static`, `/assets`, and `/favicon` without checking.
///
/// # Cache lifecycle
///
/// To avoid a per-request `SELECT 1 FROM members WHERE is_admin = 1`
/// query in steady-state operation, the middleware consults
/// `AppState::admin_exists_observed` (an `Arc<AtomicBool>`). The flag
/// starts `false` and is set to `true` the first time the middleware
/// observes any admin in the database — after that, the DB query is
/// skipped entirely for the rest of the process lifetime.
///
/// The flag is **sticky once-true**: no application-level operation
/// clears it, because no application path demotes the last admin. An
/// operator who manually removes `is_admin = 1` from every member row
/// via direct SQL will find the cache stale (the middleware will
/// continue forwarding instead of redirecting to `/setup`) until the
/// server is restarted, at which point the flag re-initializes to
/// `false` and the setup-redirect path re-arms. Any future handler that
/// can demote the last admin must invalidate the cache itself.
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

    // Fast path: once we've ever observed an admin, never query again.
    if state.admin_exists_observed.load(Ordering::Relaxed) {
        return next.run(request).await;
    }

    // Cold path: query the DB. If admin exists, arm the cache so the
    // next request skips the query.
    let has_admin = check_admin_exists(&state).await;
    if has_admin {
        state.admin_exists_observed.store(true, Ordering::Relaxed);
        return next.run(request).await;
    }

    Redirect::to("/setup").into_response()
}

/// Check if at least one admin user exists in the database
async fn check_admin_exists(state: &AppState) -> bool {
    let result: Result<Option<(i64,)>, _> =
        sqlx::query_as("SELECT 1 as exists_flag FROM members WHERE is_admin = 1 LIMIT 1")
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
