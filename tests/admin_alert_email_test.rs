//! Integration tests for `AdminAlertEmailIntegration::handle_event`
//! (`src/integrations/admin_alert_email.rs`).
//!
//! This integration's whole job is two branches that were previously
//! untested: skip when `org.contact_email` is unset, and never let a
//! send failure bubble up (a notification path must not fail the
//! originating service call). Pins the **admin-alert-email** capability's
//! "Outbound admin-alert channel" + "Recipients are configured by
//! setting" requirements.
//!
//! Drives the integration directly over the `Integration` trait with a
//! real in-memory SQLite-backed `SettingsService` and tiny in-test
//! `EmailSender` impls. Mirrors the construction in
//! `tests/auto_renew_alert_test.rs` and the direct-SQL settings seeding
//! in `tests/expiration_test.rs`.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use coterie::{
    auth::SecretCrypto,
    domain::CreateMemberRequest,
    email::{EmailMessage, EmailSender},
    error::{AppError, Result as CoterieResult},
    integrations::{admin_alert_email::AdminAlertEmailIntegration, Integration, IntegrationEvent},
    repository::{MemberRepository, SqliteMemberRepository},
    service::settings_service::SettingsService,
};
use sqlx::SqlitePool;

mod common;
use common::fresh_pool;

// ---------------------------------------------------------------------
// Test-only EmailSender impls — co-located to keep the file
// self-contained, same pattern as NoopEmailSender in
// tests/auto_renew_alert_test.rs.
// ---------------------------------------------------------------------

/// Captures every message it's handed; always succeeds.
struct RecordingSender {
    sent: Arc<Mutex<Vec<EmailMessage>>>,
}

#[async_trait]
impl EmailSender for RecordingSender {
    async fn send(&self, message: &EmailMessage) -> CoterieResult<()> {
        self.sent.lock().unwrap().push(message.clone());
        Ok(())
    }
}

/// Always fails — stands in for an SMTP timeout.
struct FailingSender;

#[async_trait]
impl EmailSender for FailingSender {
    async fn send(&self, _message: &EmailMessage) -> CoterieResult<()> {
        Err(AppError::External("smtp timeout".to_string()))
    }
}

// ---------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------

fn settings_service(pool: &SqlitePool) -> Arc<SettingsService> {
    let crypto = Arc::new(SecretCrypto::new("test-secret-please-ignore"));
    Arc::new(SettingsService::new(pool.clone(), crypto))
}

/// Set `org.contact_email`. Migration 001 seeds this key to
/// `admin@example.com`, so an UPDATE is enough — and an empty string
/// drives the "skip sending" branch.
async fn set_contact_email(pool: &SqlitePool, value: &str) {
    sqlx::query("UPDATE app_settings SET value = ? WHERE key = 'org.contact_email'")
        .bind(value)
        .execute(pool)
        .await
        .expect("set org.contact_email");
}

fn alert(subject: &str, body: &str) -> IntegrationEvent {
    IntegrationEvent::AdminAlert {
        subject: subject.to_string(),
        body: body.to_string(),
    }
}

// ---------------------------------------------------------------------
// 1. Empty recipient setting -> no send
// ---------------------------------------------------------------------

#[tokio::test]
async fn admin_alert_email_skips_when_contact_email_unset() {
    let pool = fresh_pool().await;
    set_contact_email(&pool, "").await;

    let sent = Arc::new(Mutex::new(Vec::new()));
    let sender = Arc::new(RecordingSender { sent: sent.clone() });
    let integration = AdminAlertEmailIntegration::new(settings_service(&pool), sender);

    integration
        .handle_event(&alert("x", "y"))
        .await
        .expect("handle_event must return Ok when recipient is unset");

    assert!(
        sent.lock().unwrap().is_empty(),
        "sender must not be called when org.contact_email is empty"
    );
}

// ---------------------------------------------------------------------
// 2. Send failure is absorbed, not propagated
// ---------------------------------------------------------------------

#[tokio::test]
async fn admin_alert_email_send_failure_is_swallowed() {
    let pool = fresh_pool().await;
    set_contact_email(&pool, "ops@example.org").await;

    let integration =
        AdminAlertEmailIntegration::new(settings_service(&pool), Arc::new(FailingSender));

    integration
        .handle_event(&alert("subject", "body"))
        .await
        .expect("handle_event must return Ok even when the sender errors");
}

// ---------------------------------------------------------------------
// 3. Configured recipient drives the To: list
// ---------------------------------------------------------------------

#[tokio::test]
async fn admin_alert_email_sends_to_configured_recipient() {
    let pool = fresh_pool().await;
    set_contact_email(&pool, "ops@example.org").await;

    let sent = Arc::new(Mutex::new(Vec::new()));
    let sender = Arc::new(RecordingSender { sent: sent.clone() });
    let integration = AdminAlertEmailIntegration::new(settings_service(&pool), sender);

    integration
        .handle_event(&alert("subject", "body"))
        .await
        .expect("handle_event must return Ok");

    let messages = sent.lock().unwrap();
    assert_eq!(messages.len(), 1, "exactly one email should be sent");
    assert_eq!(
        messages[0].to, "ops@example.org",
        "recipient must equal the configured org.contact_email"
    );
}

// ---------------------------------------------------------------------
// 4. Non-AdminAlert events are a no-op
// ---------------------------------------------------------------------

#[tokio::test]
async fn admin_alert_email_ignores_non_alert_events() {
    let pool = fresh_pool().await;
    set_contact_email(&pool, "ops@example.org").await;

    // Any non-AdminAlert variant exercises the early `else { return }`.
    let repo = SqliteMemberRepository::new(pool.clone());
    let member = repo
        .create(CreateMemberRequest {
            email: format!("m-{}@example.com", uuid::Uuid::new_v4()),
            username: format!("u_{}", uuid::Uuid::new_v4().simple()),
            full_name: "Test Member".to_string(),
            password: "p4ssword_long_enough".to_string(),
            membership_type_id: None,
            ..Default::default()
        })
        .await
        .expect("create member");

    let sent = Arc::new(Mutex::new(Vec::new()));
    let sender = Arc::new(RecordingSender { sent: sent.clone() });
    let integration = AdminAlertEmailIntegration::new(settings_service(&pool), sender);

    integration
        .handle_event(&IntegrationEvent::MemberExpired(member))
        .await
        .expect("handle_event must return Ok for non-AdminAlert events");

    assert!(
        sent.lock().unwrap().is_empty(),
        "non-AdminAlert events must not trigger a send"
    );
}
