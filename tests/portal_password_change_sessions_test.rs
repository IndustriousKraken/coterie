//! Integration tests for `POST /portal/profile/password` and its
//! interaction with the session store. Covers the requirement in
//! `password-management/spec.md` that a successful password change
//! invalidates every existing session for the member AND re-issues a
//! fresh session cookie to the caller. A rejected change (wrong
//! current password) must leave every session intact.
//!
//! Run with: cargo test --test portal_password_change_sessions_test

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use coterie::{
    api::state::{AppState, MoneyLimiter, RateLimiter},
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
use sqlx::SqlitePool;
use tower::ServiceExt;
use uuid::Uuid;

mod common;
use common::fresh_pool;

const CURRENT_PASSWORD: &str = "p4ssword_long_enough";
const NEW_PASSWORD: &str = "N3wp4ssword_long_enough";

struct Harness {
    app: Router,
    auth_service: Arc<AuthService>,
    csrf_service: Arc<CsrfService>,
    member_id: Uuid,
}

async fn build_harness() -> Harness {
    let pool: SqlitePool = fresh_pool().await;

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
    let event_repo: Arc<dyn EventRepository> = Arc::new(SqliteEventRepository::new(pool.clone()));
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

    let money_limiter = MoneyLimiter(RateLimiter::new(10, std::time::Duration::from_secs(60)));

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
        None,
        money_limiter.clone(),
        settings.server.base_url.clone(),
        pool.clone(),
    ));

    let billing_service =
        Arc::new(service_context.billing_service(None, settings.server.base_url.clone()));

    let app_state = AppState::new(
        service_context,
        None,
        None,
        billing_service,
        settings,
        Arc::new(coterie::api::middleware::bot_challenge::DisabledVerifier),
        money_limiter,
    );

    let member = member_repo
        .create(CreateMemberRequest {
            email: format!("u-{}@example.com", Uuid::new_v4()),
            username: format!("u_{}", Uuid::new_v4().simple()),
            full_name: "Test User".to_string(),
            password: CURRENT_PASSWORD.to_string(),
            membership_type_id: None,
            ..Default::default()
        })
        .await
        .expect("create member");
    member_repo
        .update(
            member.id,
            UpdateMemberRequest {
                status: Some(MemberStatus::Active),
                ..Default::default()
            },
        )
        .await
        .expect("activate member");

    // `require_setup` redirects every request to /setup until it has
    // observed an admin. The test member isn't an admin (and shouldn't
    // be — admin status is irrelevant to the password-change flow), so
    // flip the cache flag directly instead of seeding an admin row.
    app_state
        .admin_exists_observed
        .store(true, std::sync::atomic::Ordering::Relaxed);

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
        auth_service,
        csrf_service,
        member_id: member.id,
    }
}

/// Build an `application/x-www-form-urlencoded` body for the
/// password-change endpoint. The CSRF middleware reads the
/// `csrf_token` field directly off the body.
fn form_body(csrf_token: &str, current: &str, new: &str, confirm: &str) -> String {
    let mut parts = Vec::new();
    for (k, v) in [
        ("csrf_token", csrf_token),
        ("current_password", current),
        ("new_password", new),
        ("confirm_password", confirm),
    ] {
        parts.push(format!(
            "{}={}",
            urlencode(k),
            urlencode(v)
        ));
    }
    parts.join("&")
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Pull the `session=...` value out of the response's Set-Cookie
/// headers. Returns `None` if no `session` cookie was set.
fn extract_session_cookie(headers: &axum::http::HeaderMap) -> Option<String> {
    for v in headers.get_all(header::SET_COOKIE).iter() {
        let s = v.to_str().ok()?;
        if let Some(rest) = s.strip_prefix("session=") {
            let value = rest.split(';').next().unwrap_or("").to_string();
            return Some(value);
        }
    }
    None
}

/// Build a POST request to `/portal/profile/password` carrying the
/// supplied session cookie + form body. The CSRF middleware will
/// inspect `csrf_token` directly off the body.
fn password_change_request(session_token: &str, body: String) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/portal/profile/password")
        .header(header::COOKIE, format!("session={}", session_token))
        .header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(Body::from(body))
        .unwrap()
}

/// Mint a session for `member_id` via `AuthService::create_session` and
/// return `(session_id, token)`. The id is needed to generate a CSRF
/// token bound to it; the token is what goes in the cookie.
async fn mint_session(auth_service: &AuthService, member_id: Uuid) -> (String, String) {
    let (session, token) = auth_service
        .create_session(member_id, 24)
        .await
        .expect("create session");
    (session.id, token)
}

#[tokio::test]
async fn password_change_invalidates_other_sessions() {
    let h = build_harness().await;

    // Two real sessions: A is the caller's; B is "the other device".
    let (session_a_id, token_a) = mint_session(&h.auth_service, h.member_id).await;
    let (_session_b_id, token_b) = mint_session(&h.auth_service, h.member_id).await;

    // Sanity: both sessions validate before the change.
    assert!(
        h.auth_service
            .validate_session(&token_a)
            .await
            .expect("validate A")
            .is_some(),
        "session A should validate before the change"
    );
    assert!(
        h.auth_service
            .validate_session(&token_b)
            .await
            .expect("validate B")
            .is_some(),
        "session B should validate before the change"
    );

    // CSRF token is bound to the session being used to drive the request.
    let csrf_token = h
        .csrf_service
        .generate_token(&session_a_id)
        .await
        .expect("generate csrf token");
    let body = form_body(&csrf_token, CURRENT_PASSWORD, NEW_PASSWORD, NEW_PASSWORD);

    let resp = h
        .app
        .clone()
        .oneshot(password_change_request(&token_a, body))
        .await
        .expect("password change response");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "password change with a valid current password must succeed"
    );

    // The response MUST carry a fresh session cookie so the caller
    // stays signed in on this device.
    let new_token_a = extract_session_cookie(resp.headers())
        .expect("response must set a new session cookie");
    assert!(
        !new_token_a.is_empty(),
        "freshly issued session cookie value must be non-empty"
    );
    assert_ne!(
        new_token_a, token_a,
        "the new session cookie value must differ from the pre-change cookie"
    );

    // Pre-change tokens are dead — the invalidation sweep killed every
    // session row for the member, including the caller's. The only
    // working session for this member is the freshly minted one.
    assert!(
        h.auth_service
            .validate_session(&token_a)
            .await
            .expect("validate A after")
            .is_none(),
        "session A's PRE-change token must no longer validate"
    );
    assert!(
        h.auth_service
            .validate_session(&token_b)
            .await
            .expect("validate B after")
            .is_none(),
        "session B (other device) must no longer validate after the change"
    );
    let new_session = h
        .auth_service
        .validate_session(&new_token_a)
        .await
        .expect("validate new A")
        .expect("new session cookie must validate");
    assert_eq!(
        new_session.member_id, h.member_id,
        "new session must belong to the caller"
    );
}

#[tokio::test]
async fn password_change_with_wrong_current_does_not_touch_sessions() {
    let h = build_harness().await;

    let (session_a_id, token_a) = mint_session(&h.auth_service, h.member_id).await;
    let (_session_b_id, token_b) = mint_session(&h.auth_service, h.member_id).await;

    let csrf_token = h
        .csrf_service
        .generate_token(&session_a_id)
        .await
        .expect("generate csrf token");
    let body = form_body(
        &csrf_token,
        "definitely-not-the-current-password",
        NEW_PASSWORD,
        NEW_PASSWORD,
    );

    let resp = h
        .app
        .clone()
        .oneshot(password_change_request(&token_a, body))
        .await
        .expect("password change response");

    // The handler renders an inline error fragment with HTTP 200 (HTMX
    // expects 2xx to swap the content). Whichever status we land on,
    // the critical assertion is that no Set-Cookie session was emitted
    // and both pre-change sessions still validate.
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "rejected change still returns an HTML fragment"
    );
    assert!(
        extract_session_cookie(resp.headers()).is_none(),
        "no new session cookie may be issued when current_password is wrong"
    );

    assert!(
        h.auth_service
            .validate_session(&token_a)
            .await
            .expect("validate A")
            .is_some(),
        "session A must still validate after a rejected change"
    );
    assert!(
        h.auth_service
            .validate_session(&token_b)
            .await
            .expect("validate B")
            .is_some(),
        "session B must still validate after a rejected change"
    );
}
