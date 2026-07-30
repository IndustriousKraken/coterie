//! Integration tests for the `auth-logging` capability.
//!
//! Two invariants carry the whole change and are asserted here rather
//! than trusted:
//!
//! 1. **The log is more specific than the response.** `unknown_email`
//!    and `bad_password` must be distinguishable in the log and
//!    indistinguishable to the caller.
//! 2. **No credential reaches a log.** Asserted against a captured
//!    subscriber, including the path where a password is typed into the
//!    identifier field.
//!
//! Plus the placement fix that made any of it visible: `TraceLayer` must
//! sit on the merged router, exactly once, so portal routes are traced.
//!
//! Run with: cargo test --test auth_logging_test

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
    Router,
};
use coterie::{
    api::state::AppState,
    domain::{CreateMemberRequest, MemberStatus, UpdateMemberRequest},
};
use sqlx::SqlitePool;
use tower::ServiceExt;
use tracing_subscriber::layer::SubscriberExt;
use uuid::Uuid;

mod common;
use common::{build_app_state, fresh_pool, merged_router};

const PASSWORD: &str = "p4ssword_long_enough";

// ---------------------------------------------------------------------
// Captured subscriber
// ---------------------------------------------------------------------

/// One captured event or span, flattened to `field -> rendered value`.
#[derive(Clone, Debug)]
struct Record {
    level: tracing::Level,
    fields: HashMap<String, String>,
}

impl Record {
    fn f(&self, key: &str) -> &str {
        self.fields.get(key).map(String::as_str).unwrap_or("")
    }

    /// Every value on the record, for "does any field leak X" assertions.
    fn contains(&self, needle: &str) -> bool {
        self.fields.values().any(|v| v.contains(needle))
    }
}

#[derive(Clone, Default)]
struct Capture {
    events: Arc<Mutex<Vec<Record>>>,
    spans: Arc<Mutex<Vec<Record>>>,
}

impl Capture {
    fn events(&self) -> Vec<Record> {
        self.events.lock().unwrap().clone()
    }

    fn spans(&self) -> Vec<Record> {
        self.spans.lock().unwrap().clone()
    }

    /// Every `auth.*` event with the given `event` field.
    fn auth(&self, event: &str) -> Vec<Record> {
        self.events()
            .into_iter()
            .filter(|r| r.f("event") == event)
            .collect()
    }

    fn only(&self, event: &str) -> Record {
        let found = self.auth(event);
        assert_eq!(
            found.len(),
            1,
            "expected exactly one {event} event, got {found:#?}"
        );
        found.into_iter().next().unwrap()
    }
}

#[derive(Default)]
struct Visitor(HashMap<String, String>);

impl tracing::field::Visit for Visitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.0.insert(field.name().to_string(), value.to_string());
    }
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_string(), format!("{value:?}"));
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Capture {
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        _id: &tracing::span::Id,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = Visitor::default();
        attrs.record(&mut visitor);
        visitor
            .0
            .insert("__name".to_string(), attrs.metadata().name().to_string());
        self.spans.lock().unwrap().push(Record {
            level: *attrs.metadata().level(),
            fields: visitor.0,
        });
    }

    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = Visitor::default();
        event.record(&mut visitor);
        self.events.lock().unwrap().push(Record {
            level: *event.metadata().level(),
            fields: visitor.0,
        });
    }
}

/// Install a capturing subscriber for the current thread. `#[tokio::test]`
/// runs on a current-thread runtime, so everything the test awaits is
/// polled on this thread and lands in the capture.
///
/// Keep the returned guard alive for the body of the test.
fn capture() -> (Capture, tracing::subscriber::DefaultGuard) {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());
    let guard = tracing::subscriber::set_default(subscriber);
    (capture, guard)
}

// ---------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------

async fn harness() -> (SqlitePool, AppState, Router) {
    let pool = fresh_pool().await;
    let state = build_app_state(pool.clone()).await;
    // `require_setup` redirects everything to /setup until it has seen an
    // admin; none of these flows are about first-boot.
    state
        .admin_exists_observed
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let app = merged_router(state.clone());
    (pool, state, app)
}

async fn active_member(state: &AppState, password: &str) -> (Uuid, String) {
    let suffix = Uuid::new_v4();
    let email = format!("u-{suffix}@example.com");
    let member = state
        .service_context
        .member_repo
        .create(CreateMemberRequest {
            email: email.clone(),
            username: format!("u_{}", suffix.simple()),
            full_name: "Test User".to_string(),
            password: password.to_string(),
            membership_type_id: None,
            ..Default::default()
        })
        .await
        .expect("create member");
    state
        .service_context
        .member_repo
        .update(
            member.id,
            UpdateMemberRequest {
                status: Some(MemberStatus::Active),
                ..Default::default()
            },
        )
        .await
        .expect("activate member");
    (member.id, email)
}

async fn post_login(app: &Router, username: &str, password: &str) -> (StatusCode, String) {
    let body = serde_json::json!({ "username": username, "password": password }).to_string();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

async fn post_form(app: &Router, uri: &str, body: String) -> (StatusCode, String) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

async fn audit_count(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM audit_logs")
        .fetch_one(pool)
        .await
        .expect("count audit_logs")
}

// ---------------------------------------------------------------------
// 0. Layer placement — everything else is invisible without it
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_portal_route_produces_a_request_scoped_log_event() {
    let (capture, _guard) = capture();
    let (_pool, _state, app) = harness().await;

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/login")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // The request span carries method + path...
    let spans: Vec<_> = capture
        .spans()
        .into_iter()
        .filter(|s| s.f("__name") == "request")
        .collect();
    assert_eq!(
        spans.len(),
        1,
        "GET /login must be traced exactly once — a portal route with zero \
         spans means the layer sits before the merge again; two means it is \
         applied twice. Got: {spans:#?}"
    );
    assert_eq!(spans[0].f("method"), "GET");
    assert_eq!(spans[0].f("uri"), "/login");

    // ...and the response event carries the status.
    let finished: Vec<_> = capture
        .events()
        .into_iter()
        .filter(|e| e.f("message").contains("finished processing request"))
        .collect();
    assert_eq!(
        finished.len(),
        1,
        "expected one response event: {finished:#?}"
    );
    assert!(
        finished[0].fields.contains_key("status"),
        "the response event must carry the status: {:#?}",
        finished[0]
    );
}

#[test]
fn the_trace_layer_is_applied_once_and_after_the_merge() {
    // The behavioural test above runs against `common::merged_router`, a
    // mirror of main.rs — so it cannot see someone moving the layer back
    // inside `create_app` while tidying. This can.
    let main_rs = include_str!("../src/main.rs");
    let api_mod = include_str!("../src/api/mod.rs");

    assert_eq!(
        main_rs.matches("TraceLayer::new_for_http()").count(),
        1,
        "TraceLayer must appear exactly once in main.rs — twice means every \
         request is logged twice"
    );
    assert!(
        !api_mod.contains("TraceLayer"),
        "TraceLayer must NOT be applied inside api::create_app: in axum 0.7 a \
         layer added before Router::merge does not reach the merged portal \
         routes, so it would silently stop logging /login and /portal/*"
    );

    let merge = main_rs
        .find(".merge(web_app)")
        .expect("main.rs should still merge the web router");
    let trace = main_rs
        .find(".layer(TraceLayer::new_for_http())")
        .expect("main.rs should still apply the trace layer");
    assert!(
        trace > merge,
        "the trace layer must be applied AFTER api_app.merge(web_app)"
    );
}

// ---------------------------------------------------------------------
// 0b. Client-IP resolution visibility
// ---------------------------------------------------------------------

fn server_config(
    base_url: &str,
    secure: Option<bool>,
    trust: Option<bool>,
) -> coterie::config::ServerConfig {
    coterie::config::ServerConfig {
        host: "127.0.0.1".to_string(),
        port: 0,
        base_url: base_url.to_string(),
        data_dir: "./data".to_string(),
        uploads_dir: None,
        secure_cookies: secure,
        cors_origins: None,
        trust_forwarded_for: trust,
    }
}

#[test]
fn startup_reports_inferred_modes_with_their_provenance() {
    let (capture, _guard) = capture();
    server_config("https://example.org", None, None).log_resolved_modes();

    let lines: Vec<String> = capture
        .events()
        .into_iter()
        .map(|e| e.f("message").to_string())
        .collect();
    let all = lines.join("\n");

    assert!(
        all.contains("TRUSTED") && all.contains("inferred from base URL scheme"),
        "an https base URL should report forwarded headers trusted BY INFERENCE: {all}"
    );
    assert!(
        all.contains("Secure cookies: true (inferred from base URL scheme)"),
        "the Secure flag must report how it resolved, not only what it resolved to: {all}"
    );
    assert!(
        !all.contains("SINGLE SHARED BUCKET"),
        "trusted forwarded headers means per-IP bucketing is intact: {all}"
    );
}

#[test]
fn startup_distinguishes_explicit_configuration_from_inference() {
    let (capture, _guard) = capture();
    server_config("https://example.org", Some(true), Some(true)).log_resolved_modes();

    let all = capture
        .events()
        .into_iter()
        .map(|e| e.f("message").to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        all.contains("Secure cookies: true (explicitly configured)"),
        "same value, different provenance — an operator auditing the deployment \
         needs to tell these apart: {all}"
    );
    assert!(all.contains("(explicitly configured)"), "{all}");
}

#[test]
fn a_collapsed_rate_limit_bucket_is_warned_about() {
    let (capture, _guard) = capture();
    server_config("http://127.0.0.1:3000", None, None).log_resolved_modes();

    let warnings: Vec<_> = capture
        .events()
        .into_iter()
        .filter(|e| e.level == tracing::Level::WARN)
        .collect();
    assert_eq!(warnings.len(), 1, "expected one warning: {warnings:#?}");
    let msg = warnings[0].f("message").to_string();
    assert!(
        msg.contains("SINGLE SHARED BUCKET"),
        "the warning must name the consequence — one budget for every caller: {msg}"
    );
}

// ---------------------------------------------------------------------
// 7.1 / 2.2  The log distinguishes what the response hides
// ---------------------------------------------------------------------

#[tokio::test]
async fn unknown_email_and_bad_password_log_differently_and_respond_identically() {
    let (capture, _guard) = capture();
    let (_pool, state, app) = harness().await;
    let (member_id, email) = active_member(&state, PASSWORD).await;

    let (unknown_status, unknown_body) =
        post_login(&app, "nobody@example.com", "Wr0ngPassword!!").await;
    let (bad_status, bad_body) = post_login(&app, &email, "Wr0ngPassword!!").await;

    // The caller learns nothing.
    assert_eq!(unknown_status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        (unknown_status, unknown_body.as_str()),
        (bad_status, bad_body.as_str()),
        "an unknown email and a wrong password must be byte-identical to the caller"
    );

    // The operator learns everything.
    let events = capture.auth("auth.login");
    let reasons: Vec<String> = events.iter().map(|e| e.f("reason").to_string()).collect();
    assert_eq!(reasons, vec!["unknown_email", "bad_password"]);

    assert!(
        events.iter().all(|e| e.level == tracing::Level::WARN),
        "denials belong at warn so they surface in a warning scan"
    );
    assert_eq!(
        events[1].f("member_id"),
        member_id.to_string(),
        "a wrong password on a KNOWN account should name the member"
    );
    assert_eq!(events[0].f("identifier"), "nobody@example.com");
}

#[tokio::test]
async fn a_suspended_members_denial_names_the_status_and_the_member() {
    let (capture, _guard) = capture();
    let (_pool, state, app) = harness().await;
    let (member_id, email) = active_member(&state, PASSWORD).await;
    state
        .service_context
        .member_repo
        .update(
            member_id,
            UpdateMemberRequest {
                status: Some(MemberStatus::Suspended),
                ..Default::default()
            },
        )
        .await
        .expect("suspend");

    let (status, _) = post_login(&app, &email, PASSWORD).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let event = capture.only("auth.login");
    assert_eq!(event.f("reason"), "inactive_status");
    assert_eq!(event.f("member_id"), member_id.to_string());
    assert_eq!(event.f("detail"), "suspended");
}

#[tokio::test]
async fn a_successful_login_is_recorded_at_info_with_its_member() {
    let (capture, _guard) = capture();
    let (_pool, state, app) = harness().await;
    let (member_id, email) = active_member(&state, PASSWORD).await;

    let (status, _) = post_login(&app, &email, PASSWORD).await;
    assert_eq!(status, StatusCode::OK);

    let event = capture.only("auth.login");
    assert_eq!(event.level, tracing::Level::INFO);
    assert_eq!(event.f("outcome"), "ok");
    assert_eq!(event.f("member_id"), member_id.to_string());
    assert_eq!(event.f("ip"), "127.0.0.1");

    // The fixation sweep is its own event, so "why was I logged out" is
    // answerable from the log alone.
    assert_eq!(
        capture.only("auth.sessions_invalidated").f("member_id"),
        member_id.to_string()
    );
}

// ---------------------------------------------------------------------
// 7.2  No credential reaches a log
// ---------------------------------------------------------------------

#[tokio::test]
async fn no_log_event_contains_a_submitted_password() {
    const SECRET: &str = "Zx9-NeverInTheLogs-Passphrase";

    let (capture, _guard) = capture();
    let (_pool, state, app) = harness().await;
    let (_id, email) = active_member(&state, SECRET).await;

    // A failure, a success, and the hazard case: the password typed into
    // the identifier field.
    post_login(&app, &email, "Wr0ngPassword!!").await;
    post_login(&app, &email, SECRET).await;
    post_login(&app, SECRET, SECRET).await;
    post_form(
        &app,
        "/forgot-password",
        format!("email={}", urlencode(SECRET)),
    )
    .await;

    for record in capture.events() {
        assert!(
            !record.contains(SECRET),
            "a submitted password reached the log: {record:#?}"
        );
    }

    // ...and the malformed identifier was redacted rather than dropped,
    // so the operator still sees that an attempt happened.
    let redacted: Vec<_> = capture
        .events()
        .into_iter()
        .filter(|e| e.f("identifier") == coterie::util::auth_log::REDACTED_IDENTIFIER)
        .collect();
    assert!(
        redacted.len() >= 2,
        "the login and the reset request with a non-email identifier should \
         both log a redacted placeholder, got {redacted:#?}"
    );
}

// ---------------------------------------------------------------------
// 7.3  Attacker-controlled volume never reaches the database
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_burst_of_failed_logins_writes_no_audit_rows() {
    let (capture, _guard) = capture();
    let (pool, state, app) = harness().await;
    let (_id, email) = active_member(&state, PASSWORD).await;

    let before = audit_count(&pool).await;
    // Six attempts against a five-per-window budget: five denials plus a
    // rate-limit trip, which must also stay out of the database.
    for _ in 0..6 {
        post_login(&app, &email, "Wr0ngPassword!!").await;
    }
    assert_eq!(
        audit_count(&pool).await,
        before,
        "failed logins and rate-limit trips are log-only — a DB row per attempt \
         hands an anonymous caller a write-amplification lever"
    );

    assert_eq!(capture.auth("auth.login").len(), 5);
    let limited = capture.only("auth.rate_limited");
    assert_eq!(limited.f("reason"), "rate_limited");
    assert_eq!(limited.f("ip"), "127.0.0.1");
    assert_eq!(
        limited.f("detail"),
        "auth.login",
        "the rate-limit event must name the endpoint class it fired for"
    );
}

// ---------------------------------------------------------------------
// 7.5  The reset flow is diagnosable stage by stage
// ---------------------------------------------------------------------

async fn reset_token(pool: &SqlitePool, member_id: Uuid, ttl: chrono::Duration) -> String {
    coterie::auth::email_tokens::create_password_reset_token(pool, member_id, ttl)
        .await
        .expect("create reset token")
        .token
}

fn reset_body(token: &str, password: &str) -> String {
    format!(
        "token={}&new_password={}&confirm_password={}",
        urlencode(token),
        urlencode(password),
        urlencode(password)
    )
}

#[tokio::test]
async fn reset_token_validity_reuse_and_expiry_are_three_distinct_outcomes() {
    let (capture, _guard) = capture();
    let (pool, state, app) = harness().await;
    let (member_id, _email) = active_member(&state, PASSWORD).await;

    let good = reset_token(&pool, member_id, chrono::Duration::hours(1)).await;
    let expired = reset_token(&pool, member_id, chrono::Duration::seconds(-1)).await;
    let new_password = "N3wp4ssword_long_enough";

    post_form(&app, "/reset-password", reset_body(&good, new_password)).await;
    post_form(&app, "/reset-password", reset_body(&good, new_password)).await;
    post_form(&app, "/reset-password", reset_body(&expired, new_password)).await;
    post_form(
        &app,
        "/reset-password",
        reset_body("nonsense", new_password),
    )
    .await;

    let outcomes: Vec<String> = capture
        .auth("auth.password_reset_completed")
        .iter()
        .map(|e| format!("{}/{}", e.f("outcome"), e.f("reason")))
        .collect();
    assert_eq!(
        outcomes,
        vec![
            "ok/-",
            "denied/token_already_used",
            "denied/token_expired",
            "denied/token_unknown",
        ],
        "'the reset link didn't work' has several distinct causes and the \
         member can't tell them apart — the log must"
    );

    // A completed reset IS reviewable account history, unlike the denials.
    let rows = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM audit_logs WHERE action = 'password_reset'",
    )
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(rows, 1, "exactly one audit row for the one completed reset");
}

#[tokio::test]
async fn a_reset_for_an_unknown_address_is_logged_but_not_revealed() {
    let (capture, _guard) = capture();
    let (_pool, state, app) = harness().await;
    let (_id, email) = active_member(&state, PASSWORD).await;

    let (known_status, known_body) = post_form(
        &app,
        "/forgot-password",
        format!("email={}", urlencode(&email)),
    )
    .await;
    let (unknown_status, unknown_body) = post_form(
        &app,
        "/forgot-password",
        "email=nobody%40example.com".to_string(),
    )
    .await;

    assert_eq!(known_status, unknown_status);
    assert_eq!(
        known_body, unknown_body,
        "the reset form must answer identically for a known and an unknown address"
    );

    let events = capture.auth("auth.password_reset_requested");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].f("outcome"), "ok");
    assert_eq!(events[1].f("reason"), "unknown_email");
    assert_eq!(events[1].f("identifier"), "nobody@example.com");
}

// ---------------------------------------------------------------------
// 5.2  Policy rejections carry the rule and the length, never the value
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_rejected_password_logs_its_rule_and_length_only() {
    let (capture, _guard) = capture();
    let (pool, state, app) = harness().await;
    let (member_id, _email) = active_member(&state, PASSWORD).await;
    let token = reset_token(&pool, member_id, chrono::Duration::hours(1)).await;

    let weak = "short1A";
    let before = audit_count(&pool).await;
    post_form(&app, "/reset-password", reset_body(&token, weak)).await;

    let event = capture.only("auth.password_rejected");
    assert_eq!(event.level, tracing::Level::WARN);
    assert_eq!(event.f("reason"), "too_short");
    assert_eq!(event.f("length"), weak.len().to_string());
    assert!(
        !event.contains(weak),
        "the password itself must not be logged"
    );
    assert_eq!(
        audit_count(&pool).await,
        before,
        "a policy rejection is attacker-controlled volume: log only, no audit row"
    );
}

// ---------------------------------------------------------------------
// 7.4  Account-state changes are audited exactly once
// ---------------------------------------------------------------------

/// Log in for real and return the `session=` cookie value plus a CSRF
/// token bound to that session — the portal flows below need both.
async fn session_and_csrf(state: &AppState, member_id: Uuid) -> (String, String) {
    let (session, token) = state
        .service_context
        .auth_service
        .create_session(member_id, 24)
        .await
        .expect("create session");
    let csrf = state
        .service_context
        .csrf_service
        .generate_token(&session.id)
        .await
        .expect("csrf token");
    (token, csrf)
}

async fn post_portal_form(
    app: &Router,
    uri: &str,
    session: &str,
    body: String,
) -> (StatusCode, String) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::COOKIE, format!("session={session}"))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

#[tokio::test]
async fn a_password_change_writes_exactly_one_credential_free_audit_row() {
    let (capture, _guard) = capture();
    let (pool, state, app) = harness().await;
    let (member_id, _email) = active_member(&state, PASSWORD).await;
    let (session, csrf) = session_and_csrf(&state, member_id).await;

    const NEW: &str = "N3wp4ssword_long_enough";
    let body = format!(
        "csrf_token={}&current_password={}&new_password={}&confirm_password={}",
        urlencode(&csrf),
        urlencode(PASSWORD),
        urlencode(NEW),
        urlencode(NEW)
    );
    let (status, _) = post_portal_form(&app, "/portal/profile/password", &session, body).await;
    assert_eq!(status, StatusCode::OK);

    let rows: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT action, new_value FROM audit_logs WHERE action = 'password_change'")
            .fetch_all(&pool)
            .await
            .expect("query audit rows");
    assert_eq!(rows.len(), 1, "exactly one audit row per password change");
    assert!(
        !rows[0].1.as_deref().unwrap_or("").contains(NEW),
        "the audit row must not carry the new credential"
    );

    let event = capture.only("auth.password_changed");
    assert_eq!(event.f("member_id"), member_id.to_string());
    assert!(!event.contains(NEW) && !event.contains(PASSWORD));
}

#[tokio::test]
async fn disabling_two_factor_writes_exactly_one_audit_row() {
    let (capture, _guard) = capture();
    let (pool, state, app) = harness().await;
    let (member_id, email) = active_member(&state, PASSWORD).await;

    // Enrol first, through the service the handler uses.
    let totp_service = &state.service_context.totp_service;
    let init = totp_service.begin_enrollment(&email).expect("begin");
    let totp = {
        use totp_rs::{Algorithm, Secret, TOTP};
        TOTP::new(
            Algorithm::SHA1,
            6,
            1,
            30,
            Secret::Encoded(init.secret_base32.clone())
                .to_bytes()
                .expect("decode secret"),
            Some("Coterie".to_string()),
            email.clone(),
        )
        .expect("totp")
    };
    let code = totp.generate_current().expect("code");
    assert!(totp_service
        .confirm_enrollment(member_id, &init.secret_base32, &code, &email)
        .await
        .expect("confirm"));

    let (session, csrf) = session_and_csrf(&state, member_id).await;
    let disable_code = totp.generate_current().expect("code");
    let body = format!(
        "csrf_token={}&code={}",
        urlencode(&csrf),
        urlencode(&disable_code)
    );
    let (status, _) = post_portal_form(
        &app,
        "/portal/profile/security/totp/disable",
        &session,
        body,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let rows = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM audit_logs WHERE action = 'totp_disable'",
    )
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(
        rows, 1,
        "removing second-factor protection must be reviewable after the fact"
    );

    let event = capture.only("auth.totp_disabled");
    assert_eq!(event.f("outcome"), "ok");
    assert_eq!(event.f("member_id"), member_id.to_string());
    assert!(
        !event.contains(&disable_code),
        "the submitted TOTP code must never be logged"
    );
}
