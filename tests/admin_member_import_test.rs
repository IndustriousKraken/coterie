//! End-to-end tests for `POST /portal/admin/members/import` and its
//! GET partner. Boots the merged router with an in-memory pool, seeds
//! an admin + an active membership type with slug `regular`,
//! authenticates as the admin, and exercises:
//!
//!   - the happy path (a small valid CSV) creates members + audit rows;
//!   - the partial-failure path (one duplicate) reports the failure
//!     without aborting the batch;
//!   - the malformed-header path (missing required column) aborts the
//!     batch with no members created;
//!   - an unknown `membership_type_slug` is a per-row failure.
//!
//! Run with: cargo test --features test-utils --test admin_member_import_test

use std::sync::Arc;

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
    Router,
};
use coterie::{
    auth::{AuthService, CsrfService, PendingLoginService, SecretCrypto, TotpService},
    config::Settings,
    domain::{CreateMemberRequest, MemberStatus, UpdateMemberRequest},
    email::LogSender,
    integrations::IntegrationManager,
    repository::{
        AnnouncementRepository, EventRepository, MemberRepository, PaymentRepository,
        SqliteAnnouncementRepository, SqliteEventRepository, SqliteMemberRepository,
        SqlitePaymentRepository,
    },
    service::{settings_service::SettingsService, ServiceContext},
};
use sqlx::{Executor, SqlitePool};
use tower::ServiceExt;
use uuid::Uuid;

// ---------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------

struct Harness {
    app: Router,
    pool: SqlitePool,
    admin_id: Uuid,
    session_cookie: String,
    csrf_token: String,
}

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
    sqlx::migrate!("./migrations").run(&pool).await.expect("migrate");
    pool
}

async fn build_harness() -> Harness {
    let pool = fresh_pool().await;

    let settings = Settings {
        server: coterie::config::ServerConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            base_url: "http://127.0.0.1".to_string(),
            data_dir: "./data".to_string(),
            uploads_dir: None,
            secure_cookies: Some(false),
            cors_origins: None,
            trust_forwarded_for: Some(false),
        },
        database: coterie::config::DatabaseConfig {
            url: "sqlite::memory:".to_string(),
            max_connections: 1,
        },
        auth: coterie::config::AuthConfig {
            session_secret: "test-session-secret-please-ignore".to_string(),
            session_duration_hours: 24,
            totp_issuer: "Coterie Test".to_string(),
        },
        stripe: Default::default(),
        integrations: Default::default(),
        seed: Default::default(),
        bot_challenge: Default::default(),
    };
    let settings = Arc::new(settings);

    let member_repo: Arc<dyn MemberRepository> =
        Arc::new(SqliteMemberRepository::new(pool.clone()));
    let event_repo: Arc<dyn EventRepository> =
        Arc::new(SqliteEventRepository::new(pool.clone()));
    let announcement_repo: Arc<dyn AnnouncementRepository> =
        Arc::new(SqliteAnnouncementRepository::new(pool.clone()));
    let payment_repo: Arc<dyn PaymentRepository> =
        Arc::new(SqlitePaymentRepository::new(pool.clone()));

    let crypto = Arc::new(SecretCrypto::new("test-secret-please-ignore"));
    let auth_service = Arc::new(AuthService::new(
        pool.clone(),
        settings.auth.session_secret.clone(),
    ));
    let csrf_service = Arc::new(CsrfService::new(&settings.auth.session_secret));
    let totp_service = Arc::new(TotpService::new(
        pool.clone(),
        crypto.clone(),
        "Coterie".to_string(),
    ));
    let pending_login_service = Arc::new(PendingLoginService::new(pool.clone()));
    let settings_service = Arc::new(SettingsService::new(pool.clone(), crypto));

    let email_sender = Arc::new(LogSender::new(
        "test@example.com".to_string(),
        "Test".to_string(),
    ));
    let integration_manager = Arc::new(IntegrationManager::new());

    let service_context = Arc::new(ServiceContext::new(
        member_repo.clone(),
        event_repo,
        announcement_repo,
        payment_repo,
        integration_manager,
        auth_service.clone(),
        email_sender,
        settings_service,
        csrf_service.clone(),
        totp_service,
        pending_login_service,
        settings.server.base_url.clone(),
        pool.clone(),
    ));

    let billing_service = Arc::new(service_context.billing_service(
        None,
        settings.server.base_url.clone(),
    ));

    let app_state = coterie::api::state::AppState::new(
        service_context,
        None,
        None,
        billing_service,
        settings,
        Arc::new(coterie::api::middleware::bot_challenge::DisabledVerifier),
    );

    // Seed an active membership type with the slug "regular". The
    // bootstrap migration ships `member` / `associate` / `life-member`
    // — add ours alongside them so the CSVs in this test file can
    // reference it directly.
    sqlx::query(
        "INSERT INTO membership_types \
         (id, name, slug, description, color, icon, sort_order, is_active, fee_cents, billing_period, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, NULL, 10, 1, 0, 'monthly', datetime('now'), datetime('now'))",
    )
    .bind(Uuid::new_v4().to_string())
    .bind("Regular")
    .bind("regular")
    .bind("Regular membership for testing")
    .bind("#000000")
    .execute(&pool)
    .await
    .expect("seed regular membership type");

    // Create an admin: Active + is_admin = 1. require_admin_redirect
    // accepts Active or Honorary admins.
    let admin = member_repo
        .create(CreateMemberRequest {
            email: "admin@example.com".to_string(),
            username: "admin".to_string(),
            full_name: "Admin User".to_string(),
            password: "p4ssword_long_enough".to_string(),
            membership_type_id: None,
        })
        .await
        .expect("create admin member");
    member_repo
        .update(
            admin.id,
            UpdateMemberRequest {
                status: Some(MemberStatus::Active),
                ..Default::default()
            },
        )
        .await
        .expect("activate admin");
    member_repo
        .set_admin(admin.id, true)
        .await
        .expect("promote admin");

    let (session, token) = auth_service
        .create_session(admin.id, 24)
        .await
        .expect("create session");
    let session_cookie = format!("session={}", token);
    let csrf_token = csrf_service
        .generate_token(&session.id)
        .await
        .expect("generate csrf token");

    let api_app = coterie::api::create_app(app_state.clone());
    let web_app = coterie::web::create_web_routes(app_state.clone());

    let app = api_app
        .merge(web_app)
        .layer(axum::middleware::from_fn_with_state(
            app_state.clone(),
            coterie::api::middleware::setup::require_setup,
        ))
        .layer(axum::middleware::from_fn_with_state(
            app_state,
            coterie::api::middleware::security::csrf_protect_unless_exempt,
        ));

    Harness {
        app,
        pool,
        admin_id: admin.id,
        session_cookie,
        csrf_token,
    }
}

/// Build a multipart/form-data body with a csrf_token text field and
/// a `file` field carrying the supplied CSV bytes under the supplied
/// filename. The CSRF middleware reads `csrf_token` first, so it must
/// appear before `file` in the body.
fn build_multipart(csrf_token: &str, file_name: &str, csv_bytes: &[u8])
    -> (String, Vec<u8>)
{
    let boundary = "----coterie-test-boundary-xyz";
    let mut body: Vec<u8> = Vec::new();

    // csrf_token field
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Disposition: form-data; name=\"csrf_token\"\r\n\r\n");
    body.extend_from_slice(csrf_token.as_bytes());
    body.extend_from_slice(b"\r\n");

    // file field
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"file\"; filename=\"{file_name}\"\r\n"
        ).as_bytes(),
    );
    body.extend_from_slice(b"Content-Type: text/csv\r\n\r\n");
    body.extend_from_slice(csv_bytes);
    body.extend_from_slice(b"\r\n");

    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());

    let content_type = format!("multipart/form-data; boundary={boundary}");
    (content_type, body)
}

fn import_request(h: &Harness, file_name: &str, csv: &[u8]) -> Request<Body> {
    let (ct, body) = build_multipart(&h.csrf_token, file_name, csv);
    Request::builder()
        .method("POST")
        .uri("/portal/admin/members/import")
        .header(header::COOKIE, &h.session_cookie)
        .header(header::CONTENT_TYPE, ct)
        .body(Body::from(body))
        .unwrap()
}

async fn member_count(pool: &SqlitePool) -> i64 {
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM members")
        .fetch_one(pool)
        .await
        .unwrap();
    row.0
}

async fn audit_count_by_action(pool: &SqlitePool, action: &str) -> i64 {
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM audit_logs WHERE action = ?",
    )
    .bind(action)
    .fetch_one(pool)
    .await
    .unwrap();
    row.0
}

async fn member_exists(pool: &SqlitePool, email: &str) -> bool {
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM members WHERE email = ?",
    )
    .bind(email)
    .fetch_one(pool)
    .await
    .unwrap();
    row.0 > 0
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[tokio::test]
async fn import_happy_path_creates_members_and_audit_rows() {
    let h = build_harness().await;

    // Baseline: just the seeded admin row.
    let before = member_count(&h.pool).await;
    assert_eq!(before, 1, "expected only the admin in members before import");

    let csv = "email,username,full_name,membership_type_slug,status\n\
               alice@example.com,alice,Alice A.,regular,Active\n\
               bob@example.com,bob,Bob B.,regular,Pending\n\
               carla@example.com,carla,Carla C.,regular,\n";

    let resp = h
        .app
        .clone()
        .oneshot(import_request(&h, "members.csv", csv.as_bytes()))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK, "import POST status");
    let body = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let text = String::from_utf8(body.to_vec()).expect("utf-8 body");
    assert!(
        text.contains("3 members imported") || text.contains(">3<"),
        "result fragment should show 3 imported; got:\n{}", text,
    );

    // 3 new members + 1 admin = 4 rows.
    assert_eq!(member_count(&h.pool).await, before + 3);
    assert!(member_exists(&h.pool, "alice@example.com").await);
    assert!(member_exists(&h.pool, "bob@example.com").await);
    assert!(member_exists(&h.pool, "carla@example.com").await);

    // 3 per-row import_member rows + 1 aggregate import_members_batch.
    assert_eq!(audit_count_by_action(&h.pool, "import_member").await, 3);
    assert_eq!(audit_count_by_action(&h.pool, "import_members_batch").await, 1);

    // Aggregate row carries the right summary string.
    let agg: (String, String, Option<String>) = sqlx::query_as(
        "SELECT entity_type, entity_id, new_value \
         FROM audit_logs WHERE action = 'import_members_batch'",
    )
    .fetch_one(&h.pool)
    .await
    .unwrap();
    assert_eq!(agg.0, "member");
    assert_eq!(agg.1, "*");
    let summary = agg.2.unwrap_or_default();
    assert!(
        summary.contains("file=members.csv")
            && summary.contains("succeeded=3")
            && summary.contains("failed=0"),
        "aggregate new_value should describe file + counts; got {:?}", summary,
    );

    // Each per-row audit row carries the new member's email.
    let per_row: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT entity_id, new_value FROM audit_logs WHERE action = 'import_member' ORDER BY entity_id",
    )
    .fetch_all(&h.pool)
    .await
    .unwrap();
    assert_eq!(per_row.len(), 3);
    let emails: Vec<String> = per_row.iter().filter_map(|r| r.1.clone()).collect();
    assert!(emails.contains(&"alice@example.com".to_string()));
    assert!(emails.contains(&"bob@example.com".to_string()));
    assert!(emails.contains(&"carla@example.com".to_string()));

    // Actor on every audit row matches the importing admin.
    let actor_rows: Vec<(Option<String>,)> = sqlx::query_as(
        "SELECT actor_id FROM audit_logs \
         WHERE action IN ('import_member','import_members_batch')",
    )
    .fetch_all(&h.pool)
    .await
    .unwrap();
    assert!(!actor_rows.is_empty());
    for (actor,) in &actor_rows {
        assert_eq!(actor.as_deref(), Some(h.admin_id.to_string().as_str()));
    }
}

#[tokio::test]
async fn import_partial_failure_reports_duplicate_email() {
    let h = build_harness().await;

    // Pre-seed an existing member with the duplicate email. The import
    // tries to re-create them — should fail that one row only.
    let repo = SqliteMemberRepository::new(h.pool.clone());
    repo.create(CreateMemberRequest {
        email: "dup@example.com".to_string(),
        username: "dup_existing".to_string(),
        full_name: "Existing Dup".to_string(),
        password: "p4ssword_long_enough".to_string(),
        membership_type_id: None,
    })
    .await
    .unwrap();
    let before = member_count(&h.pool).await; // admin + dup_existing = 2.

    let csv = "email,username,full_name,membership_type_slug\n\
               new1@example.com,n1,New One,regular\n\
               dup@example.com,n2,Dup Attempt,regular\n\
               new2@example.com,n3,New Two,regular\n";

    let resp = h
        .app
        .clone()
        .oneshot(import_request(&h, "partial.csv", csv.as_bytes()))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();

    // 2 succeeded, 1 failed.
    assert_eq!(member_count(&h.pool).await, before + 2);
    assert!(member_exists(&h.pool, "new1@example.com").await);
    assert!(member_exists(&h.pool, "new2@example.com").await);

    // Fragment lists the duplicate-email failure with the email value.
    assert!(
        text.contains("dup@example.com"),
        "result fragment must mention the duplicate email; got:\n{}", text,
    );
    assert!(
        text.to_lowercase().contains("already exists"),
        "result fragment must call out the duplicate; got:\n{}", text,
    );

    // 2 successful import_member rows + 1 aggregate.
    assert_eq!(audit_count_by_action(&h.pool, "import_member").await, 2);
    assert_eq!(audit_count_by_action(&h.pool, "import_members_batch").await, 1);
    let agg_summary: (Option<String>,) = sqlx::query_as(
        "SELECT new_value FROM audit_logs WHERE action = 'import_members_batch'",
    )
    .fetch_one(&h.pool)
    .await
    .unwrap();
    let s = agg_summary.0.unwrap_or_default();
    assert!(s.contains("succeeded=2") && s.contains("failed=1"), "agg: {:?}", s);
}

#[tokio::test]
async fn import_missing_required_column_aborts_batch() {
    let h = build_harness().await;
    let before = member_count(&h.pool).await;

    // Header is missing `email` — required column. Abort the batch:
    // no rows should be created.
    let csv = "username,full_name,membership_type_slug\n\
               alice,Alice A.,regular\n\
               bob,Bob B.,regular\n";

    let resp = h
        .app
        .clone()
        .oneshot(import_request(&h, "bad-header.csv", csv.as_bytes()))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();

    // The error fragment should call out the missing column. The
    // batch must not create members or audit rows.
    assert!(
        text.contains("email") && text.to_lowercase().contains("missing"),
        "expected missing-column error message; got:\n{}", text,
    );
    assert_eq!(member_count(&h.pool).await, before);
    assert_eq!(audit_count_by_action(&h.pool, "import_member").await, 0);
    assert_eq!(audit_count_by_action(&h.pool, "import_members_batch").await, 0);
}

#[tokio::test]
async fn import_unknown_membership_slug_fails_only_that_row() {
    let h = build_harness().await;
    let before = member_count(&h.pool).await;

    let csv = "email,username,full_name,membership_type_slug\n\
               ok@example.com,ok_user,OK User,regular\n\
               weird@example.com,weird,Weird User,not-a-real-slug\n";

    let resp = h
        .app
        .clone()
        .oneshot(import_request(&h, "unknown-slug.csv", csv.as_bytes()))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();

    // ok@example.com made it; weird@ did not.
    assert_eq!(member_count(&h.pool).await, before + 1);
    assert!(member_exists(&h.pool, "ok@example.com").await);
    assert!(!member_exists(&h.pool, "weird@example.com").await);

    // Result fragment names the unknown slug.
    assert!(
        text.contains("not-a-real-slug"),
        "expected unknown slug to appear in failure list; got:\n{}", text,
    );
}
