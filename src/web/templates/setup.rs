use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use askama::Template;
use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio::sync::Mutex as AsyncMutex;

use crate::{
    domain::{CreateMemberRequest, MemberStatus, UpdateMemberRequest},
    repository::MemberRepository,
    service::settings_service::SettingsService,
    web::templates::{BaseContext, HtmlTemplate},
};

#[derive(Template)]
#[template(path = "auth/setup.html")]
pub struct SetupTemplate {
    pub base: BaseContext,
}

#[derive(Debug, Deserialize)]
pub struct SetupRequest {
    pub org_name: String,
    pub email: String,
    pub username: String,
    pub full_name: String,
    pub password: String,
    pub password_confirm: String,
}

#[derive(Debug, Serialize)]
pub struct SetupResponse {
    pub success: bool,
    pub redirect: Option<String>,
    pub error: Option<String>,
}

// GET /setup
//
// Defense-in-depth: once an admin exists, /setup is a dead-end. The
// POST handler already refuses inside the setup_lock; the GET handler
// redirects to /login so an operator who stumbles here post-bootstrap
// doesn't see a form they can't submit.
//
// Reads `admin_exists_observed` first to match the `require_setup`
// middleware's cache contract — once any path observes an admin, every
// subsequent request skips the DB query for the rest of the process.
pub async fn setup_page(
    State(admin_exists_observed): State<Arc<AtomicBool>>,
    State(db_pool): State<SqlitePool>,
) -> Response {
    if admin_exists_observed.load(Ordering::Relaxed) {
        return redirect_to_login();
    }

    if check_admin_exists(&db_pool).await {
        // Arm the flag for consistency with the middleware's cache
        // semantics — subsequent requests skip the redundant query.
        admin_exists_observed.store(true, Ordering::Relaxed);
        return redirect_to_login();
    }

    let template = SetupTemplate {
        base: BaseContext::for_anon(),
    };
    HtmlTemplate(template).into_response()
}

fn redirect_to_login() -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(header::LOCATION, "/login".parse().unwrap());
    (StatusCode::SEE_OTHER, headers).into_response()
}

// POST /setup
//
// Setup is intrinsically cross-cutting: it touches the lock, the
// admin-observed flag, the member repo (create + update + set_admin),
// the settings service, and the DB pool. Granular extraction per D1.
pub async fn setup_handler(
    State(setup_lock): State<Arc<AsyncMutex<()>>>,
    State(admin_exists_observed): State<Arc<AtomicBool>>,
    State(db_pool): State<SqlitePool>,
    State(member_repo): State<Arc<dyn MemberRepository>>,
    State(settings_service): State<Arc<SettingsService>>,
    Json(request): Json<SetupRequest>,
) -> Response {
    // Validate inputs before acquiring the setup lock so failed requests
    // don't hold the lock while the caller fixes and retries.
    if request.password != request.password_confirm {
        return (
            StatusCode::BAD_REQUEST,
            Json(SetupResponse {
                success: false,
                redirect: None,
                error: Some("Passwords do not match".to_string()),
            }),
        )
            .into_response();
    }

    if let Err(msg) = crate::auth::validate_password(&request.password) {
        return (
            StatusCode::BAD_REQUEST,
            Json(SetupResponse {
                success: false,
                redirect: None,
                error: Some(msg.to_string()),
            }),
        )
            .into_response();
    }

    if !request.email.contains('@') {
        return (
            StatusCode::BAD_REQUEST,
            Json(SetupResponse {
                success: false,
                redirect: None,
                error: Some("Invalid email address".to_string()),
            }),
        )
            .into_response();
    }

    // Serialize first-admin creation. Without this, two concurrent setup
    // requests can both pass the "no admin exists" check and both create
    // admin accounts. The lock is held across check + create + promote.
    let _setup_guard = setup_lock.lock().await;

    // Re-check inside the lock using the authoritative is_admin column
    // (not the legacy notes-LIKE heuristic).
    if check_admin_exists(&db_pool).await {
        return (
            StatusCode::BAD_REQUEST,
            Json(SetupResponse {
                success: false,
                redirect: None,
                error: Some("Setup has already been completed".to_string()),
            }),
        )
            .into_response();
    }

    // Create the admin member. Membership type defaults to the first
    // active row (migration 001 seeds three; an org doing a clean
    // install with all three deleted would need to add one before
    // setup, but the wizard runs before any admin tooling exists).
    let create_request = CreateMemberRequest {
        email: request.email.clone(),
        username: request.username.clone(),
        full_name: request.full_name.clone(),
        password: request.password.clone(),
        membership_type_id: None,
        ..Default::default()
    };

    let member = match member_repo.create(create_request).await {
        Ok(m) => m,
        Err(e) => {
            tracing::error!("Failed to create admin user: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(SetupResponse {
                    success: false,
                    redirect: None,
                    error: Some(format!("Failed to create admin user: {}", e)),
                }),
            )
                .into_response();
        }
    };

    // Promote to Active with bypass_dues
    let update_request = UpdateMemberRequest {
        status: Some(MemberStatus::Active),
        bypass_dues: Some(true),
        ..Default::default()
    };

    if let Err(e) = member_repo.update(member.id, update_request).await {
        tracing::error!("Failed to activate admin user: {}", e);
    }

    // Set is_admin = 1 (the authoritative admin flag, used by middleware)
    if let Err(e) = member_repo.set_admin(member.id, true).await {
        tracing::error!("Failed to set is_admin on admin user: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(SetupResponse {
                success: false,
                redirect: None,
                error: Some("Failed to promote user to admin".to_string()),
            }),
        )
            .into_response();
    }

    // Proactively arm the middleware cache so the very next request
    // skips the redundant `SELECT 1 FROM members WHERE is_admin = 1`
    // round-trip. Without this, the middleware would learn this same
    // fact via its own DB query on the next call.
    admin_exists_observed.store(true, Ordering::Relaxed);

    // Persist the org name to the org.name setting so it shows up in
    // emails, banners, and the public site. Soft-fail: setup itself
    // already succeeded, the admin can edit org.name later if this
    // doesn't take.
    let org_name = request.org_name.trim();
    if !org_name.is_empty() {
        let update = crate::domain::UpdateSettingRequest {
            value: org_name.to_string(),
            reason: Some("Set during initial setup".to_string()),
        };
        if let Err(e) = settings_service
            .update_setting("org.name", update, member.id)
            .await
        {
            tracing::warn!(
                "Couldn't persist org.name during setup ({}); admin can edit later",
                e
            );
        }
    }
    tracing::info!("Setup complete for organization: {}", request.org_name);

    let mut headers = HeaderMap::new();
    headers.insert("HX-Redirect", "/login".parse().unwrap());

    (
        StatusCode::OK,
        headers,
        Json(SetupResponse {
            success: true,
            redirect: Some("/login".to_string()),
            error: None,
        }),
    )
        .into_response()
}

/// Check if at least one admin user exists in the database.
/// Uses the `is_admin` column — the authoritative source.
async fn check_admin_exists(db_pool: &SqlitePool) -> bool {
    let result: Result<Option<(i64,)>, _> =
        sqlx::query_as("SELECT 1 as exists_flag FROM members WHERE is_admin = 1 LIMIT 1")
            .fetch_optional(db_pool)
            .await;

    match result {
        Ok(Some(_)) => true,
        Ok(None) => false,
        Err(e) => {
            tracing::error!("Failed to check for admin: {}", e);
            false
        }
    }
}
