//! Integration tests for `Notifications::send_event_reminders` —
//! the runner method that emails members who RSVP'd to an upcoming
//! event. Hits a real in-memory SQLite + migrations and asserts on
//! both the side-effect (one email queued via a fake sender) and the
//! claim semantics (`event_attendance.reminder_sent_at` stamped).
//!
//! Run: cargo test --features test-utils --test event_reminder_test

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use coterie::{
    auth::SecretCrypto,
    domain::{CreateMemberRequest, Event, EventType, EventVisibility},
    email::{EmailMessage, EmailSender, LogSender},
    error::{AppError, Result as CoterieResult},
    integrations::IntegrationManager,
    repository::{
        EventRepository, MemberRepository, SqliteEventRepository, SqliteMemberRepository,
        SqliteSavedCardRepository, SqliteScheduledPaymentRepository, SqlitePaymentRepository,
    },
    service::{
        billing_service::BillingService,
        membership_type_service::MembershipTypeService,
        settings_service::SettingsService,
    },
};
use sqlx::{Executor, SqlitePool};
use tokio::sync::Mutex;
use uuid::Uuid;

// ---------------------------------------------------------------------
// Fake EmailSender — records every message; can be flipped to error.
// ---------------------------------------------------------------------

struct FakeEmailSender {
    sent: Mutex<Vec<EmailMessage>>,
    fail: bool,
}

impl FakeEmailSender {
    fn ok() -> Arc<Self> {
        Arc::new(Self { sent: Mutex::new(Vec::new()), fail: false })
    }
    fn failing() -> Arc<Self> {
        Arc::new(Self { sent: Mutex::new(Vec::new()), fail: true })
    }
    async fn count(&self) -> usize {
        self.sent.lock().await.len()
    }
    async fn first(&self) -> Option<EmailMessage> {
        self.sent.lock().await.first().cloned()
    }
}

#[async_trait]
impl EmailSender for FakeEmailSender {
    async fn send(&self, message: &EmailMessage) -> CoterieResult<()> {
        if self.fail {
            return Err(AppError::Internal("fake sender configured to fail".into()));
        }
        self.sent.lock().await.push(message.clone());
        Ok(())
    }
}

// ---------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------

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

struct H {
    pool: SqlitePool,
    billing: BillingService,
    event_repo: Arc<dyn EventRepository>,
    member: Uuid,
    event_id: Uuid,
}

/// Build a harness with the email sender of choice. Pre-seeds a
/// member, an event at `event_start`, and an attendance row in the
/// given `status` (typically "Registered").
async fn build_with(
    email: Arc<FakeEmailSender>,
    event_start: DateTime<Utc>,
    status: &str,
) -> H {
    let pool = fresh_pool().await;

    let member_repo: Arc<dyn MemberRepository> =
        Arc::new(SqliteMemberRepository::new(pool.clone()));
    let event_repo: Arc<dyn EventRepository> =
        Arc::new(SqliteEventRepository::new(pool.clone()));
    let payment_repo = Arc::new(SqlitePaymentRepository::new(pool.clone()));
    let saved_card_repo = Arc::new(SqliteSavedCardRepository::new(pool.clone()));
    let scheduled_repo = Arc::new(SqliteScheduledPaymentRepository::new(pool.clone()));
    let mt_repo = Arc::new(coterie::repository::SqliteMembershipTypeRepository::new(pool.clone()));
    let mt_service = Arc::new(MembershipTypeService::new(mt_repo));
    let crypto = Arc::new(SecretCrypto::new("test-secret-please-ignore"));
    let settings = Arc::new(SettingsService::new(pool.clone(), crypto));
    let integrations = Arc::new(IntegrationManager::new());

    // Inject the fake email sender via the EmailSender trait object —
    // matching what BillingService expects.
    let email_for_billing: Arc<dyn EmailSender> = email.clone();

    let billing = BillingService::new(
        scheduled_repo,
        payment_repo,
        saved_card_repo,
        member_repo.clone(),
        event_repo.clone(),
        mt_service,
        settings,
        email_for_billing,
        integrations,
        None,
        "http://localhost:3000".to_string(),
        pool.clone(),
    );

    // Seed a member.
    let member = member_repo
        .create(CreateMemberRequest {
            email: "rsvp@example.com".to_string(),
            username: format!("u_{}", Uuid::new_v4().simple()),
            full_name: "RSVP'd Member".to_string(),
            password: "p4ssword_long_enough".to_string(),
            membership_type_id: None,
        })
        .await
        .expect("create member");

    // Seed an event.
    let event = Event {
        id: Uuid::new_v4(),
        title: "Quarterly Meetup".to_string(),
        description: "It's happening.".to_string(),
        event_type: EventType::Social,
        event_type_id: None,
        visibility: EventVisibility::MembersOnly,
        start_time: event_start,
        end_time: Some(event_start + Duration::hours(2)),
        location: Some("HQ".to_string()),
        max_attendees: None,
        rsvp_required: true,
        image_url: None,
        created_by: member.id,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        series_id: None,
        occurrence_index: None,
    };
    let event = event_repo.create(event).await.expect("create event");

    // Seed the attendance row directly so we can pick the status.
    sqlx::query(
        r#"
        INSERT INTO event_attendance (event_id, member_id, status, registered_at)
        VALUES (?, ?, ?, CURRENT_TIMESTAMP)
        "#,
    )
    .bind(event.id.to_string())
    .bind(member.id.to_string())
    .bind(status)
    .execute(&pool)
    .await
    .expect("seed attendance");

    let _ = email; // returned through the caller's local clone
    H {
        pool,
        billing,
        event_repo,
        member: member.id,
        event_id: event.id,
    }
}

async fn reminder_sent_at(pool: &SqlitePool, event_id: Uuid, member_id: Uuid) -> Option<String> {
    let row: Option<(Option<String>,)> = sqlx::query_as(
        "SELECT reminder_sent_at FROM event_attendance WHERE event_id = ? AND member_id = ?",
    )
    .bind(event_id.to_string())
    .bind(member_id.to_string())
    .fetch_optional(pool)
    .await
    .expect("query");
    row.and_then(|(v,)| v)
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[tokio::test]
async fn event_in_window_sends_email_and_stamps_row() {
    let email = FakeEmailSender::ok();
    let h = build_with(email.clone(), Utc::now() + Duration::hours(6), "Registered").await;

    let sent = h
        .billing
        .notifications
        .send_event_reminders()
        .await
        .expect("send");

    assert_eq!(sent, 1, "exactly one email should have been sent");
    assert_eq!(email.count().await, 1);
    let msg = email.first().await.unwrap();
    assert_eq!(msg.to, "rsvp@example.com");
    assert!(msg.subject.contains("Quarterly Meetup"), "subject was: {:?}", msg.subject);
    assert!(msg.subject.starts_with("Reminder:"));
    assert!(reminder_sent_at(&h.pool, h.event_id, h.member).await.is_some());
}

#[tokio::test]
async fn event_outside_window_skipped() {
    let email = FakeEmailSender::ok();
    // Default lead is 24h; an event in 48h is well outside.
    let h = build_with(email.clone(), Utc::now() + Duration::hours(48), "Registered").await;

    let sent = h
        .billing
        .notifications
        .send_event_reminders()
        .await
        .expect("send");

    assert_eq!(sent, 0);
    assert_eq!(email.count().await, 0);
    assert!(reminder_sent_at(&h.pool, h.event_id, h.member).await.is_none());
}

#[tokio::test]
async fn already_stamped_row_skipped() {
    let email = FakeEmailSender::ok();
    let h = build_with(email.clone(), Utc::now() + Duration::hours(6), "Registered").await;

    // Pre-stamp the row.
    sqlx::query(
        "UPDATE event_attendance SET reminder_sent_at = CURRENT_TIMESTAMP \
         WHERE event_id = ? AND member_id = ?",
    )
    .bind(h.event_id.to_string())
    .bind(h.member.to_string())
    .execute(&h.pool)
    .await
    .expect("pre-stamp");
    let before = reminder_sent_at(&h.pool, h.event_id, h.member).await;
    assert!(before.is_some());

    let sent = h
        .billing
        .notifications
        .send_event_reminders()
        .await
        .expect("send");

    assert_eq!(sent, 0);
    assert_eq!(email.count().await, 0);
}

#[tokio::test]
async fn send_failure_keeps_row_stamped() {
    // Per the claim-then-send policy: row is claimed FIRST, then we
    // send. If the send fails, the stamp stays in place — operator
    // intervention required to retry.
    let email = FakeEmailSender::failing();
    let h = build_with(email.clone(), Utc::now() + Duration::hours(6), "Registered").await;

    let sent = h
        .billing
        .notifications
        .send_event_reminders()
        .await
        .expect("call returns Ok even when individual send errors");

    assert_eq!(sent, 0, "no successful sends");
    assert_eq!(email.count().await, 0, "fake sender records nothing on failure");
    // The row IS stamped — that's the documented trade-off.
    assert!(
        reminder_sent_at(&h.pool, h.event_id, h.member).await.is_some(),
        "row should stay stamped after a failed send",
    );

    // And the underlying repo's mark_reminder_sent now returns false
    // (already claimed), proving the claim was persisted.
    let claim_again = h
        .event_repo
        .mark_reminder_sent(h.event_id, h.member)
        .await
        .expect("mark");
    assert!(!claim_again);
}

// LogSender import kept for parity with neighbour test files; suppresses
// the dead-code lint when nothing in this module wires the real sender.
#[allow(dead_code)]
fn _silence_logsender_import(_: &LogSender) {}
