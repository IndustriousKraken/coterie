//! Integration tests for `Notifications::send_dues_reminders` — the daily
//! dues-reminder router (`src/service/billing_service/notifications.rs`).
//!
//! Each test hits a real in-memory SQLite + migrations and a `Notifications`
//! wired with a recording `EmailSender`, so the production routing executes
//! end-to-end. The four routing cases (manual / auto-renew-monthly /
//! auto-renew-yearly / auto-renew-card-invalid), the lifetime skip, and the
//! per-cycle idempotency flag are each asserted.
//!
//! Run with: cargo test --features test-utils --test dues_reminder_test

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Datelike, Duration, Utc};
use coterie::{
    auth::SecretCrypto,
    domain::{CreateMemberRequest, SavedCard},
    email::{EmailMessage, EmailSender},
    error::Result as CoterieResult,
    integrations::IntegrationManager,
    repository::{
        EventRepository, MemberRepository, SavedCardRepository, SqliteEventRepository,
        SqliteMemberRepository, SqliteMembershipTypeRepository, SqliteSavedCardRepository,
    },
    service::{
        billing_service::notifications::Notifications,
        membership_type_service::MembershipTypeService, settings_service::SettingsService,
    },
};
use sqlx::SqlitePool;
use uuid::Uuid;

mod common;
use common::fresh_pool;

// ---------------------------------------------------------------------
// RecordingEmailSender — test-only EmailSender that captures every sent
// message into a shared Vec the test can inspect. Returns Ok(()) always.
// ---------------------------------------------------------------------

struct RecordingEmailSender {
    messages: Arc<Mutex<Vec<EmailMessage>>>,
}

#[async_trait]
impl EmailSender for RecordingEmailSender {
    async fn send(&self, message: &EmailMessage) -> CoterieResult<()> {
        self.messages.lock().unwrap().push(message.clone());
        Ok(())
    }
}

// ---------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------

struct Harness {
    pool: SqlitePool,
    notifications: Notifications,
    saved_card_repo: Arc<SqliteSavedCardRepository>,
    messages: Arc<Mutex<Vec<EmailMessage>>>,
}

async fn build_harness() -> Harness {
    let pool = fresh_pool().await;

    let member_repo: Arc<dyn MemberRepository> =
        Arc::new(SqliteMemberRepository::new(pool.clone()));
    let event_repo: Arc<dyn EventRepository> = Arc::new(SqliteEventRepository::new(pool.clone()));
    let saved_card_repo = Arc::new(SqliteSavedCardRepository::new(pool.clone()));
    let mt_repo = Arc::new(SqliteMembershipTypeRepository::new(pool.clone()));
    let mt_service = Arc::new(MembershipTypeService::new(mt_repo));
    let crypto = Arc::new(SecretCrypto::new("test-secret-please-ignore"));
    let settings = Arc::new(SettingsService::new(pool.clone(), crypto));

    let messages: Arc<Mutex<Vec<EmailMessage>>> = Arc::new(Mutex::new(Vec::new()));
    let email: Arc<dyn EmailSender> = Arc::new(RecordingEmailSender {
        messages: messages.clone(),
    });

    let notifications = Notifications::new(
        member_repo,
        saved_card_repo.clone() as Arc<dyn SavedCardRepository>,
        event_repo,
        mt_service,
        settings,
        email,
        Arc::new(IntegrationManager::new()),
        "http://localhost:3000".to_string(),
        pool.clone(),
    );

    Harness {
        pool,
        notifications,
        saved_card_repo,
        messages,
    }
}

fn recorded(messages: &Arc<Mutex<Vec<EmailMessage>>>) -> Vec<EmailMessage> {
    messages.lock().unwrap().clone()
}

// ---------------------------------------------------------------------
// Seed helpers
// ---------------------------------------------------------------------

/// Id of a seeded membership type by slug. Migration 001 seeds 'member'
/// (monthly) and 'life-member' (lifetime).
async fn mt_id_for_slug(pool: &SqlitePool, slug: &str) -> String {
    sqlx::query_scalar("SELECT id FROM membership_types WHERE slug = ? LIMIT 1")
        .bind(slug)
        .fetch_one(pool)
        .await
        .expect("seeded membership_type id")
}

/// No yearly type is seeded, so create one for the renewal-notice case.
async fn seed_yearly_type(pool: &SqlitePool) -> String {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO membership_types \
         (id, name, slug, description, color, icon, sort_order, is_active, fee_cents, \
          billing_period, created_at, updated_at) \
         VALUES (?, 'Yearly Test', 'yearly-test', '', NULL, NULL, 99, 1, 12000, 'yearly', \
                 datetime('now'), datetime('now'))",
    )
    .bind(&id)
    .execute(pool)
    .await
    .expect("insert yearly membership_type");
    id
}

/// Create an Active member on `mt_id` with the given billing mode.
async fn seed_member(pool: &SqlitePool, billing_mode: &str, mt_id: &str) -> Uuid {
    let repo = SqliteMemberRepository::new(pool.clone());
    let member = repo
        .create(CreateMemberRequest {
            email: format!("m-{}@example.com", Uuid::new_v4()),
            username: format!("u_{}", Uuid::new_v4().simple()),
            full_name: "Test Member".to_string(),
            password: "p4ssword_long_enough".to_string(),
            membership_type_id: None,
            ..Default::default()
        })
        .await
        .expect("create member");

    sqlx::query("UPDATE members SET billing_mode = ?, membership_type_id = ? WHERE id = ?")
        .bind(billing_mode)
        .bind(mt_id)
        .bind(member.id.to_string())
        .execute(pool)
        .await
        .expect("stamp billing_mode + membership_type");
    member.id
}

/// Put the member inside the reminder window: dues `days_from_now` out,
/// Active, not bypassing dues, reminder flag cleared; and pin the window
/// setting to 7 days (so a 3-day-out member is included).
async fn set_dues_window(pool: &SqlitePool, member_id: Uuid, days_from_now: i64) {
    let due = Utc::now() + Duration::days(days_from_now);
    sqlx::query(
        "UPDATE members SET status = 'Active', bypass_dues = 0, \
         dues_reminder_sent_at = NULL, dues_paid_until = ? WHERE id = ?",
    )
    .bind(due)
    .bind(member_id.to_string())
    .execute(pool)
    .await
    .expect("set dues window");

    sqlx::query(
        "UPDATE app_settings SET value = '7' WHERE key = 'membership.reminder_days_before'",
    )
    .execute(pool)
    .await
    .expect("set reminder_days_before");
}

/// Default card for the member. `valid` controls expiry: far-future when
/// true, last year (expired) when false — drives the case-4 branch.
async fn seed_default_card(repo: &Arc<SqliteSavedCardRepository>, member_id: Uuid, valid: bool) {
    let now = Utc::now();
    let card = SavedCard {
        id: Uuid::new_v4(),
        member_id,
        stripe_payment_method_id: format!("pm_test_{}", Uuid::new_v4()),
        card_last_four: "4242".to_string(),
        card_brand: "visa".to_string(),
        exp_month: 12,
        exp_year: if valid {
            now.year() + 5
        } else {
            now.year() - 1
        },
        is_default: true,
        fingerprint: None,
        created_at: now,
        updated_at: now,
    };
    repo.create(card).await.expect("create card");
}

async fn reminder_flag(pool: &SqlitePool, member_id: Uuid) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>(
        "SELECT dues_reminder_sent_at FROM members WHERE id = ?",
    )
    .bind(member_id.to_string())
    .fetch_one(pool)
    .await
    .expect("query dues_reminder_sent_at")
}

// The card-invalid callout phrase, present in both the HTML and text
// reminder templates only inside the `{% if card_invalid %}` block.
const CARD_INVALID_CALLOUT: &str = "charge it automatically";

// ---------------------------------------------------------------------
// Case 1 — manual billing sends a plain reminder
// ---------------------------------------------------------------------

#[tokio::test]
async fn manual_member_gets_plain_reminder() {
    let h = build_harness().await;
    let mt = mt_id_for_slug(&h.pool, "member").await; // monthly, non-lifetime
    let member_id = seed_member(&h.pool, "manual", &mt).await;
    set_dues_window(&h.pool, member_id, 3).await;

    let sent = h.notifications.send_dues_reminders().await.expect("run");
    assert_eq!(sent, 1, "manual member in window must get one reminder");

    let msgs = recorded(&h.messages);
    assert_eq!(msgs.len(), 1, "exactly one email recorded");
    let m = &msgs[0];
    assert!(
        m.subject.contains("dues are due soon"),
        "subject must be the reminder subject, got: {}",
        m.subject
    );
    assert!(
        !m.text_body.contains(CARD_INVALID_CALLOUT) && !m.html_body.contains(CARD_INVALID_CALLOUT),
        "manual reminder must NOT include the card-invalid callout"
    );
}

// ---------------------------------------------------------------------
// Case 2 — auto-renew + valid card + monthly is skipped and stays eligible
// ---------------------------------------------------------------------

#[tokio::test]
async fn autorenew_monthly_valid_card_is_skipped() {
    let h = build_harness().await;
    let mt = mt_id_for_slug(&h.pool, "member").await; // monthly
    let member_id = seed_member(&h.pool, "coterie_managed", &mt).await;
    set_dues_window(&h.pool, member_id, 3).await;
    seed_default_card(&h.saved_card_repo, member_id, true).await;

    let sent = h.notifications.send_dues_reminders().await.expect("run");
    assert_eq!(sent, 0, "auto-renew monthly w/ valid card sends nothing");
    assert!(
        recorded(&h.messages).is_empty(),
        "no email should be recorded for the skipped member"
    );
}

#[tokio::test]
async fn autorenew_monthly_skip_does_not_set_reminder_flag() {
    let h = build_harness().await;
    let mt = mt_id_for_slug(&h.pool, "member").await; // monthly
    let member_id = seed_member(&h.pool, "coterie_managed", &mt).await;
    set_dues_window(&h.pool, member_id, 3).await;
    seed_default_card(&h.saved_card_repo, member_id, true).await;

    h.notifications.send_dues_reminders().await.expect("run");

    assert!(
        reminder_flag(&h.pool, member_id).await.is_none(),
        "case-2 skip must leave dues_reminder_sent_at NULL so the member \
         stays eligible if their card or mode changes mid-window"
    );
}

// ---------------------------------------------------------------------
// Case 3 — auto-renew + valid card + yearly gets a renewal notice
// ---------------------------------------------------------------------

#[tokio::test]
async fn autorenew_yearly_valid_card_gets_renewal_notice() {
    let h = build_harness().await;
    let mt = seed_yearly_type(&h.pool).await;
    let member_id = seed_member(&h.pool, "coterie_managed", &mt).await;
    set_dues_window(&h.pool, member_id, 3).await;
    seed_default_card(&h.saved_card_repo, member_id, true).await;

    let sent = h.notifications.send_dues_reminders().await.expect("run");
    assert_eq!(sent, 1, "auto-renew yearly w/ valid card gets a notice");

    let msgs = recorded(&h.messages);
    assert_eq!(msgs.len(), 1, "exactly one email recorded");
    assert!(
        msgs[0].subject.contains("will renew"),
        "subject must be the renewal notice, got: {}",
        msgs[0].subject
    );
    assert!(
        !msgs[0].subject.contains("dues are due soon"),
        "renewal notice must be distinct from the plain reminder subject"
    );
}

// ---------------------------------------------------------------------
// Case 4 — auto-renew + invalid/missing card gets reminder with callout
// ---------------------------------------------------------------------

#[tokio::test]
async fn autorenew_expired_card_gets_reminder_with_card_invalid_callout() {
    let h = build_harness().await;
    let mt = mt_id_for_slug(&h.pool, "member").await; // monthly, non-lifetime
    let member_id = seed_member(&h.pool, "coterie_managed", &mt).await;
    set_dues_window(&h.pool, member_id, 3).await;
    seed_default_card(&h.saved_card_repo, member_id, false).await; // expired

    let sent = h.notifications.send_dues_reminders().await.expect("run");
    assert_eq!(sent, 1, "auto-renew w/ dead card still gets warned");

    let msgs = recorded(&h.messages);
    assert_eq!(msgs.len(), 1, "exactly one email recorded");
    assert!(
        msgs[0].subject.contains("dues are due soon"),
        "subject must be the reminder subject, got: {}",
        msgs[0].subject
    );
    assert!(
        msgs[0].text_body.contains(CARD_INVALID_CALLOUT)
            || msgs[0].html_body.contains(CARD_INVALID_CALLOUT),
        "reminder for a dead-card auto-renew member must include the \
         card-invalid callout"
    );
}

// ---------------------------------------------------------------------
// Lifetime members are skipped
// ---------------------------------------------------------------------

#[tokio::test]
async fn lifetime_member_is_skipped() {
    let h = build_harness().await;
    let mt = mt_id_for_slug(&h.pool, "life-member").await; // lifetime
    let member_id = seed_member(&h.pool, "manual", &mt).await;
    set_dues_window(&h.pool, member_id, 3).await;

    let sent = h.notifications.send_dues_reminders().await.expect("run");
    assert_eq!(sent, 0, "lifetime member in window gets nothing");
    assert!(
        recorded(&h.messages).is_empty(),
        "no email should be recorded for a lifetime member"
    );
}

// ---------------------------------------------------------------------
// Idempotency within a cycle
// ---------------------------------------------------------------------

#[tokio::test]
async fn reminder_is_idempotent_within_cycle() {
    let h = build_harness().await;
    let mt = mt_id_for_slug(&h.pool, "member").await;
    let member_id = seed_member(&h.pool, "manual", &mt).await;
    set_dues_window(&h.pool, member_id, 3).await;

    let first = h.notifications.send_dues_reminders().await.expect("run 1");
    assert_eq!(first, 1, "first run sends the reminder");
    assert_eq!(recorded(&h.messages).len(), 1, "one email after first run");

    // Second run, no reset of dues_reminder_sent_at: the
    // `dues_reminder_sent_at IS NULL` filter must exclude the member.
    let second = h.notifications.send_dues_reminders().await.expect("run 2");
    assert_eq!(second, 0, "second run sends nothing for the same member");
    assert_eq!(
        recorded(&h.messages).len(),
        1,
        "no additional email recorded on the idempotent second run"
    );
}
