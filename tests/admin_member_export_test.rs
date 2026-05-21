//! End-to-end test for `GET /portal/admin/members/export` — the CSV
//! download added by the `a12-bulk-member-csv-export` change. Boots
//! the merged router with an in-memory pool, seeds a handful of
//! members across different statuses, authenticates as an admin via
//! a real session cookie, and asserts:
//!
//!   - the export returns 200 with the right MIME / disposition;
//!   - the CSV header matches the spec's column order exactly;
//!   - the body contains one row per seeded member (no filter), or
//!     just the Active rows when filtering;
//!   - RFC 4180 escaping holds for fields with commas / quotes;
//!   - a successful export writes an `audit_logs` row with
//!     `action = "export_members"` for the acting admin.
//!
//! Run with: cargo test --features test-utils --test admin_member_export_test

use std::sync::Arc;

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
    Router,
};
use coterie::{
    api::state::{MoneyLimiter, RateLimiter},
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
    member_repo: Arc<dyn MemberRepository>,
    admin_id: Uuid,
    session_cookie: String,
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

    let money_limiter = MoneyLimiter(RateLimiter::new(
        10,
        std::time::Duration::from_secs(60),
    ));

    let service_context = Arc::new(ServiceContext::new(
        member_repo.clone(),
        event_repo,
        announcement_repo,
        payment_repo,
        integration_manager,
        auth_service.clone(),
        email_sender,
        settings_service,
        csrf_service,
        totp_service,
        pending_login_service,
        None, // stripe_client not needed for these tests
        money_limiter.clone(),
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
        money_limiter,
    );

    // Create an admin member: Active status, is_admin = 1. The
    // `require_admin_redirect` gate accepts only Active or Honorary
    // admins; bypass_dues isn't required.
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

    let (_session, token) = auth_service
        .create_session(admin.id, 24)
        .await
        .expect("create session");
    let session_cookie = format!("session={}", token);

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
        member_repo,
        admin_id: admin.id,
        session_cookie,
    }
}

/// Seed a member, then force-set the given status. Returns the member id.
async fn seed_member(
    h: &Harness,
    email: &str,
    username: &str,
    full_name: &str,
    status: MemberStatus,
    notes: Option<&str>,
) -> Uuid {
    let m = h
        .member_repo
        .create(CreateMemberRequest {
            email: email.to_string(),
            username: username.to_string(),
            full_name: full_name.to_string(),
            password: "p4ssword_long_enough".to_string(),
            membership_type_id: None,
        })
        .await
        .expect("create member");
    h.member_repo
        .update(
            m.id,
            UpdateMemberRequest {
                status: Some(status),
                notes: notes.map(|s| s.to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("update member status/notes");
    m.id
}

fn auth_get(h: &Harness, uri: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .header(header::COOKIE, &h.session_cookie)
        .body(Body::empty())
        .unwrap()
}

/// Tiny RFC-4180 parser: splits a single CSV row honoring quoted
/// fields and `""` → `"` un-escapes. Newlines inside fields aren't
/// expected in this test's seed data; we split rows by `\n` first.
fn parse_csv_row(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = line.chars().peekable();
    let mut in_quotes = false;
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    cur.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                cur.push(c);
            }
        } else if c == ',' {
            out.push(std::mem::take(&mut cur));
        } else if c == '"' {
            in_quotes = true;
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[tokio::test]
async fn export_unfiltered_returns_csv_with_all_members() {
    let h = build_harness().await;
    seed_member(&h, "alice@example.com", "alice", "Alice A.",
                MemberStatus::Active, None).await;
    seed_member(&h, "bob@example.com", "bob", "Bob B.",
                MemberStatus::Pending, None).await;
    seed_member(&h, "carla@example.com", "carla", "Carla C.",
                MemberStatus::Expired, None).await;

    let resp = h
        .app
        .clone()
        .oneshot(auth_get(&h, "/portal/admin/members/export"))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp.headers().get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok()).unwrap_or("");
    assert_eq!(ct, "text/csv; charset=utf-8");
    let cd = resp.headers().get(header::CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    assert!(
        cd.starts_with("attachment; filename=\"members-export-") && cd.ends_with(".csv\""),
        "expected dated attachment filename, got {:?}", cd,
    );

    let body = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let text = String::from_utf8(body.to_vec()).expect("utf-8 csv body");
    let mut lines = text.lines();
    let header = lines.next().expect("header row present");
    assert_eq!(
        header,
        "id,email,username,full_name,status,membership_type,joined_at,\
         dues_paid_until,is_admin,bypass_dues,discord_id,email_verified_at,notes",
    );
    // 3 seeded + 1 admin = 4 data rows.
    let data_rows: Vec<&str> = lines.filter(|l| !l.is_empty()).collect();
    assert_eq!(data_rows.len(), 4, "expected 4 data rows, got {}\n{}", data_rows.len(), text);
}

#[tokio::test]
async fn export_status_filter_returns_only_matching_members() {
    let h = build_harness().await;
    let active_id = seed_member(&h, "alice@example.com", "alice", "Alice A.",
                                MemberStatus::Active, None).await;
    seed_member(&h, "bob@example.com", "bob", "Bob B.",
                MemberStatus::Pending, None).await;
    seed_member(&h, "carla@example.com", "carla", "Carla C.",
                MemberStatus::Expired, None).await;

    let resp = h
        .app
        .clone()
        .oneshot(auth_get(&h, "/portal/admin/members/export?status=Active"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    let mut lines = text.lines();
    let _hdr = lines.next();
    let data_rows: Vec<&str> = lines.filter(|l| !l.is_empty()).collect();

    // Admin is Active too, so the count is 2 (Alice + Admin).
    assert_eq!(data_rows.len(), 2, "Active filter must keep only Active rows; got body:\n{}", text);
    let ids: Vec<String> = data_rows
        .iter()
        .map(|r| parse_csv_row(r).into_iter().next().unwrap())
        .collect();
    assert!(ids.iter().any(|id| id == &active_id.to_string()));
    assert!(ids.iter().any(|id| id == &h.admin_id.to_string()));
}

#[tokio::test]
async fn export_escapes_special_characters_per_rfc_4180() {
    let h = build_harness().await;
    seed_member(
        &h,
        "obrien@example.com",
        "sobrien",
        "O'Brien, Sean",
        MemberStatus::Active,
        Some("Has \"complications\""),
    ).await;

    let resp = h
        .app
        .clone()
        .oneshot(auth_get(&h, "/portal/admin/members/export"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();

    // The raw body must contain RFC 4180-style escaping: full_name
    // wrapped in quotes (it contains a comma), and notes with `""`
    // around the internal quotes.
    assert!(
        text.contains(r#""O'Brien, Sean""#),
        "expected quoted full_name to appear verbatim in body, got:\n{}", text,
    );
    assert!(
        text.contains(r#""Has ""complications""""#),
        "expected doubled-quote escaping for notes in body, got:\n{}", text,
    );

    // Round-trip: parsing the row we care about yields the original strings.
    let row_line = text.lines().find(|l| l.contains("obrien@example.com"))
        .expect("seeded row present");
    let fields = parse_csv_row(row_line);
    // Column order: id, email, username, full_name, status, ...
    assert_eq!(fields[1], "obrien@example.com");
    assert_eq!(fields[3], "O'Brien, Sean");
    // notes is the last column.
    assert_eq!(fields.last().unwrap(), "Has \"complications\"");
}

#[tokio::test]
async fn export_writes_audit_row_for_acting_admin() {
    let h = build_harness().await;
    seed_member(&h, "alice@example.com", "alice", "Alice A.",
                MemberStatus::Active, None).await;

    let resp = h
        .app
        .clone()
        .oneshot(auth_get(&h, "/portal/admin/members/export?status=Active"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let row: (i64, Option<String>, String, String, Option<String>) = sqlx::query_as(
        "SELECT COUNT(*), MAX(actor_id), MAX(entity_type), MAX(entity_id), MAX(new_value) \
         FROM audit_logs WHERE action = 'export_members'",
    )
    .fetch_one(&h.pool)
    .await
    .expect("query audit_logs");

    assert_eq!(row.0, 1, "exactly one export_members audit row expected");
    assert_eq!(row.1.as_deref(), Some(h.admin_id.to_string().as_str()));
    assert_eq!(row.2, "member");
    assert_eq!(row.3, "*");
    // new_value contains the filter summary + count: 1 seeded Active + 1 admin = 2.
    let new_value = row.4.unwrap_or_default();
    assert!(
        new_value.contains("status=Active") && new_value.ends_with("count=2"),
        "new_value should describe filter + count; got {:?}", new_value,
    );
}
