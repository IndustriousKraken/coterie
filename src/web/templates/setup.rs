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
use uuid::Uuid;

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

    match query_admin_exists(&db_pool).await {
        Ok(true) => {
            // Arm the flag for consistency with the middleware's cache
            // semantics — subsequent requests skip the redundant query.
            admin_exists_observed.store(true, Ordering::Relaxed);
            redirect_to_login()
        }
        // Unknown is treated as "an admin exists" (same as
        // `check_admin_exists`), but deliberately does NOT arm the
        // sticky flag: a transient DB error must not permanently
        // disable the wizard on a genuine fresh install.
        Err(e) => {
            tracing::error!("Failed to check for admin: {}", e);
            redirect_to_login()
        }
        Ok(false) => {
            let template = SetupTemplate {
                base: BaseContext::for_anon(),
            };
            HtmlTemplate(template).into_response()
        }
    }
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

    // ponytail: no caller IP here — the first-boot wizard runs once, before
    // any proxy-trust config is even meaningful, and pulling `Settings` in
    // just to resolve it would be plumbing for a field nobody queries.
    // Add it if setup ever becomes a repeatable, remotely-reachable flow.
    if let Err(rule) = crate::auth::validate_password_logged(&request.password, None, None) {
        return (
            StatusCode::BAD_REQUEST,
            Json(SetupResponse {
                success: false,
                redirect: None,
                error: Some(rule.message().to_string()),
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

    // Fast path: once any surface has observed an admin, the wizard is
    // closed for the rest of the process — no DB round-trip, and no way
    // for a later query error to re-open it. Not a replacement for the
    // in-lock re-check below, which still covers a cold process.
    if admin_exists_observed.load(Ordering::Relaxed) {
        return setup_already_completed();
    }

    // Serialize first-admin creation. Without this, two concurrent setup
    // requests can both pass the "no admin exists" check and both create
    // admin accounts. The lock is held across check + create + promote.
    let _setup_guard = setup_lock.lock().await;

    // Re-check inside the lock using the authoritative is_admin column
    // (not the legacy notes-LIKE heuristic).
    if check_admin_exists(&db_pool).await {
        return setup_already_completed();
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

    // Critical step, same failure semantics as set_admin below: a
    // Pending first admin is admitted by no middleware tier, so a
    // swallowed failure here silently locks the org out. Clean up the
    // partial row and abort with 500 instead of arming the cache.
    if let Err(e) = member_repo.update(member.id, update_request).await {
        tracing::error!("Failed to activate admin user: {}", e);
        cleanup_partial_admin(member_repo.as_ref(), member.id).await;
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(SetupResponse {
                success: false,
                redirect: None,
                error: Some("Failed to activate admin user".to_string()),
            }),
        )
            .into_response();
    }

    // Set is_admin = 1 (the authoritative admin flag, used by middleware)
    if let Err(e) = member_repo.set_admin(member.id, true).await {
        tracing::error!("Failed to set is_admin on admin user: {}", e);
        cleanup_partial_admin(member_repo.as_ref(), member.id).await;
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

/// The wizard's refusal response, shared by the cached fast path and
/// the in-lock DB re-check. Both mean the same thing to the caller:
/// this instance is already provisioned (or can't prove it isn't).
fn setup_already_completed() -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(SetupResponse {
            success: false,
            redirect: None,
            error: Some("Setup has already been completed".to_string()),
        }),
    )
        .into_response()
}

/// Best-effort removal of the just-created member row when a later setup
/// step fails. Leaves a retryable state: without this, the orphaned
/// `Pending` row trips the UNIQUE email/username constraint on the
/// operator's next `POST /setup`. Reuses the repo's `delete` (which
/// only touches this id); logs and ignores errors since the request is
/// already failing.
async fn cleanup_partial_admin(member_repo: &dyn MemberRepository, id: Uuid) {
    if let Err(e) = member_repo.delete(id).await {
        tracing::warn!(
            "Couldn't remove partial admin row after setup failure: {}",
            e
        );
    }
}

/// Check if at least one admin user exists in the database.
/// Uses the `is_admin` column — the authoritative source.
///
/// Fails CLOSED: an unreadable answer (any query error — pool timeout,
/// `SQLITE_BUSY`, I/O) is treated as "an admin exists", so the
/// unauthenticated, CSRF-exempt setup wizard can never be re-opened on
/// a provisioned instance by a database error. Matches
/// `src/api/middleware/setup.rs::check_admin_exists`, which makes the
/// same choice for the same reason.
async fn check_admin_exists(db_pool: &SqlitePool) -> bool {
    match query_admin_exists(db_pool).await {
        Ok(exists) => exists,
        Err(e) => {
            tracing::error!("Failed to check for admin: {}", e);
            true
        }
    }
}

/// The raw admin-existence query. Callers that arm the sticky
/// `admin_exists_observed` cache use this directly so they can tell
/// "no admin" from "couldn't tell" — only a definitive positive may
/// arm the flag.
async fn query_admin_exists(db_pool: &SqlitePool) -> Result<bool, sqlx::Error> {
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT 1 as exists_flag FROM members WHERE is_admin = 1 LIMIT 1")
            .fetch_optional(db_pool)
            .await?;
    Ok(row.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::body::to_bytes;
    use sqlx::Executor;

    use crate::{auth::SecretCrypto, repository::SqliteMemberRepository};

    /// Migrated in-memory pool, pinned to one connection (a
    /// `sqlite::memory:` DB is connection-private).
    async fn fresh_pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .after_connect(|conn, _| {
                Box::pin(async move {
                    conn.execute("PRAGMA foreign_keys = ON").await?;
                    Ok(())
                })
            })
            .connect("sqlite::memory:")
            .await
            .expect("connect to :memory:");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrate");
        pool
    }

    fn valid_request() -> SetupRequest {
        SetupRequest {
            org_name: "Test Org".to_string(),
            email: "admin@example.com".to_string(),
            username: "admin".to_string(),
            full_name: "Admin User".to_string(),
            password: "WizardPass1".to_string(),
            password_confirm: "WizardPass1".to_string(),
        }
    }

    /// Drive `POST /setup` with the handler's own extractors. The route
    /// wiring is `web::create_web_routes`' business; what matters here
    /// is the handler's gate.
    async fn post_setup(pool: &SqlitePool, observed: &Arc<AtomicBool>) -> Response {
        let member_repo: Arc<dyn MemberRepository> =
            Arc::new(SqliteMemberRepository::new(pool.clone()));
        let settings_service = Arc::new(SettingsService::new(
            pool.clone(),
            Arc::new(SecretCrypto::new("test-secret-please-ignore")),
        ));
        setup_handler(
            State(Arc::new(AsyncMutex::new(()))),
            State(observed.clone()),
            State(pool.clone()),
            State(member_repo),
            State(settings_service),
            Json(valid_request()),
        )
        .await
    }

    async fn error_field(response: Response) -> Option<String> {
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).expect("json body");
        parsed
            .get("error")
            .and_then(|e| e.as_str())
            .map(str::to_string)
    }

    async fn member_count(pool: &SqlitePool) -> i64 {
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM members")
            .fetch_one(pool)
            .await
            .expect("count members");
        count
    }

    /// Regression: the admin-existence check used to return "no admin"
    /// when its query errored, so any unauthenticated caller who could
    /// make that one SELECT fail (pool timeout, SQLITE_BUSY) could mint
    /// a second admin on a provisioned instance. Unknown now means
    /// "an admin exists".
    #[tokio::test]
    async fn setup_refuses_when_admin_check_errors() {
        let pool = fresh_pool().await;
        // Closed pool → every query returns Err, including the gate's.
        pool.close().await;

        let observed = Arc::new(AtomicBool::new(false));
        let response = post_setup(&pool, &observed).await;

        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "a query error must refuse setup, not open it"
        );
        assert!(
            error_field(response).await.is_some(),
            "refusal should carry an error message"
        );
    }

    /// The process-cached flag is authoritative on its own: once armed,
    /// the wizard refuses without touching the DB — even though this
    /// DB genuinely holds no admin row.
    #[tokio::test]
    async fn setup_refuses_when_admin_already_observed() {
        let pool = fresh_pool().await;
        let observed = Arc::new(AtomicBool::new(true));

        let response = post_setup(&pool, &observed).await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            member_count(&pool).await,
            0,
            "no member row may be created once an admin has been observed"
        );
    }

    /// First boot is unchanged: the wizard creates one Active admin and
    /// arms the cache.
    #[tokio::test]
    async fn setup_creates_first_admin() {
        let pool = fresh_pool().await;
        let observed = Arc::new(AtomicBool::new(false));

        let response = post_setup(&pool, &observed).await;
        assert_eq!(response.status(), StatusCode::OK);

        assert_eq!(member_count(&pool).await, 1);
        let (is_admin, status): (i64, String) =
            sqlx::query_as("SELECT is_admin, status FROM members")
                .fetch_one(&pool)
                .await
                .expect("fetch the created admin");
        assert_eq!(is_admin, 1);
        assert_eq!(status, "Active");
        assert!(
            observed.load(Ordering::Relaxed),
            "wizard must arm the process cache after creating the first admin"
        );
    }
}
