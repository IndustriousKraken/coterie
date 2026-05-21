//! Integration tests for `AutoRenew::process_scheduled_payment`'s
//! terminal-failure AdminAlert dispatch (a22).
//!
//! Hits a real in-memory SQLite + migrations + `BillingService` so the
//! actual production path executes end-to-end. The test
//! `RecordingIntegration` captures every `IntegrationEvent` so the
//! tests can assert which AdminAlerts fired (or didn't).
//!
//! Run with: cargo test --features test-utils --test auto_renew_alert_test

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use coterie::{
    auth::SecretCrypto,
    domain::{CreateMemberRequest, SavedCard, ScheduledPayment, ScheduledPaymentStatus},
    email::{EmailMessage, EmailSender},
    error::{AppError, Result as CoterieResult},
    integrations::{Integration, IntegrationEvent, IntegrationManager},
    payments::{
        fake_gateway::FakeStripeGateway, gateway::StripeGateway, StripeClient,
    },
    repository::{
        EventRepository, MemberRepository, PaymentRepository, SavedCardRepository,
        ScheduledPaymentRepository, SqliteEventRepository, SqliteMemberRepository,
        SqlitePaymentRepository, SqliteSavedCardRepository, SqliteScheduledPaymentRepository,
    },
    service::{
        billing_service::BillingService, membership_type_service::MembershipTypeService,
        settings_service::SettingsService,
    },
};
use sqlx::{Executor, SqlitePool};
use uuid::Uuid;

// ---------------------------------------------------------------------
// RecordingIntegration — test-only Integration impl that captures every
// dispatched IntegrationEvent into a shared Vec the test can inspect.
// Mirrors the shim in stripe_webhook_test.rs (a21). Co-located rather
// than extracted so the test file stays self-contained.
// ---------------------------------------------------------------------

struct RecordingIntegration {
    events: Arc<Mutex<Vec<IntegrationEvent>>>,
}

#[async_trait]
impl Integration for RecordingIntegration {
    fn name(&self) -> &str {
        "test-recording"
    }
    fn is_enabled(&self) -> bool {
        true
    }
    async fn health_check(&self) -> CoterieResult<()> {
        Ok(())
    }
    async fn handle_event(&self, event: &IntegrationEvent) -> CoterieResult<()> {
        self.events.lock().unwrap().push(event.clone());
        Ok(())
    }
}

fn admin_alerts(
    events: &Arc<Mutex<Vec<IntegrationEvent>>>,
) -> Vec<(String, String)> {
    events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            IntegrationEvent::AdminAlert { subject, body } => {
                Some((subject.clone(), body.clone()))
            }
            _ => None,
        })
        .collect()
}

struct NoopEmailSender;

#[async_trait]
impl EmailSender for NoopEmailSender {
    async fn send(&self, _message: &EmailMessage) -> CoterieResult<()> {
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

struct Harness {
    pool: SqlitePool,
    billing: BillingService,
    fake: Arc<FakeStripeGateway>,
    scheduled_repo: Arc<SqliteScheduledPaymentRepository>,
    saved_card_repo: Arc<SqliteSavedCardRepository>,
    recorded_events: Arc<Mutex<Vec<IntegrationEvent>>>,
}

async fn build_harness() -> Harness {
    let pool = fresh_pool().await;
    let fake = Arc::new(FakeStripeGateway::new());

    let payment_repo: Arc<dyn PaymentRepository> =
        Arc::new(SqlitePaymentRepository::new(pool.clone()));
    let member_repo: Arc<dyn MemberRepository> =
        Arc::new(SqliteMemberRepository::new(pool.clone()));
    let event_repo: Arc<dyn EventRepository> =
        Arc::new(SqliteEventRepository::new(pool.clone()));
    let scheduled_repo = Arc::new(SqliteScheduledPaymentRepository::new(pool.clone()));
    let saved_card_repo = Arc::new(SqliteSavedCardRepository::new(pool.clone()));
    let mt_repo = Arc::new(coterie::repository::SqliteMembershipTypeRepository::new(
        pool.clone(),
    ));
    let mt_service = Arc::new(MembershipTypeService::new(mt_repo));
    let crypto = Arc::new(SecretCrypto::new("test-secret-please-ignore"));
    let settings = Arc::new(SettingsService::new(pool.clone(), crypto));
    let email: Arc<dyn EmailSender> = Arc::new(NoopEmailSender);

    let integrations = Arc::new(IntegrationManager::new());
    let recorded_events: Arc<Mutex<Vec<IntegrationEvent>>> =
        Arc::new(Mutex::new(Vec::new()));
    integrations
        .register(Arc::new(RecordingIntegration {
            events: recorded_events.clone(),
        }))
        .await;

    let gw: Arc<dyn StripeGateway> = fake.clone();
    let stripe_client = Arc::new(StripeClient::with_gateway(
        gw,
        payment_repo.clone(),
        member_repo.clone(),
    ));

    let billing = BillingService::new(
        scheduled_repo.clone() as Arc<dyn ScheduledPaymentRepository>,
        payment_repo,
        saved_card_repo.clone() as Arc<dyn SavedCardRepository>,
        member_repo,
        event_repo,
        mt_service,
        settings,
        email,
        integrations,
        Some(stripe_client),
        "http://localhost:3000".to_string(),
        pool.clone(),
    );

    Harness {
        pool,
        billing,
        fake,
        scheduled_repo,
        saved_card_repo,
        recorded_events,
    }
}

// ---------------------------------------------------------------------
// Seed helpers
// ---------------------------------------------------------------------

/// Insert a coterie_managed member with the seeded 'member' membership
/// type, a stripe_customer_id (so charge_saved_card can find it), and
/// a relative `dues_paid_until` 30 days out (per a14/a15 anchor rule).
async fn seed_coterie_managed_member(pool: &SqlitePool) -> (Uuid, Uuid) {
    let repo = SqliteMemberRepository::new(pool.clone());
    let member = repo
        .create(CreateMemberRequest {
            email: format!("m-{}@example.com", Uuid::new_v4()),
            username: format!("u_{}", Uuid::new_v4().simple()),
            full_name: "Jane Smith".to_string(),
            password: "p4ssword_long_enough".to_string(),
            membership_type_id: None,
            ..Default::default()
        })
        .await
        .expect("create member");

    let mt_id: String =
        sqlx::query_scalar("SELECT id FROM membership_types WHERE slug = 'member' LIMIT 1")
            .fetch_one(pool)
            .await
            .expect("seeded 'member' membership_type");
    let mt_uuid = Uuid::parse_str(&mt_id).expect("mt uuid");

    let dues_until = Utc::now() + Duration::days(30);

    sqlx::query(
        "UPDATE members \
         SET stripe_customer_id = ?, billing_mode = 'coterie_managed', \
             membership_type_id = ?, dues_paid_until = ? \
         WHERE id = ?",
    )
    .bind(format!("cus_test_{}", member.id))
    .bind(&mt_id)
    .bind(dues_until)
    .bind(member.id.to_string())
    .execute(pool)
    .await
    .expect("stamp coterie_managed + customer + dues");

    (member.id, mt_uuid)
}

/// Insert a default SavedCard that won't expire within the test window.
async fn seed_default_card(
    saved_card_repo: &Arc<SqliteSavedCardRepository>,
    member_id: Uuid,
) {
    use chrono::Datelike;
    let now = Utc::now();
    let card = SavedCard {
        id: Uuid::new_v4(),
        member_id,
        stripe_payment_method_id: format!("pm_test_{}", Uuid::new_v4()),
        card_last_four: "4242".to_string(),
        card_brand: "visa".to_string(),
        exp_month: 12,
        exp_year: now.year() + 5,
        is_default: true,
        created_at: now,
        updated_at: now,
    };
    saved_card_repo.create(card).await.expect("create card");
}

/// Insert a Pending scheduled_payment row with the requested retry_count.
async fn seed_scheduled_payment(
    scheduled_repo: &Arc<SqliteScheduledPaymentRepository>,
    member_id: Uuid,
    membership_type_id: Uuid,
    retry_count: i32,
) -> Uuid {
    let now = Utc::now();
    let sp = ScheduledPayment {
        id: Uuid::new_v4(),
        member_id,
        membership_type_id,
        amount_cents: 50_00,
        currency: "USD".to_string(),
        due_date: now.date_naive(),
        status: ScheduledPaymentStatus::Pending,
        retry_count,
        last_attempt_at: None,
        payment_id: None,
        failure_reason: None,
        created_at: now,
        updated_at: now,
    };
    let created = scheduled_repo.create(sp).await.expect("create scheduled_payment");
    created.id
}

async fn scheduled_status(pool: &SqlitePool, id: Uuid) -> String {
    sqlx::query_scalar::<_, String>(
        "SELECT status FROM scheduled_payments WHERE id = ?",
    )
    .bind(id.to_string())
    .fetch_one(pool)
    .await
    .expect("query scheduled_payments status")
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[tokio::test]
async fn terminal_failure_dispatches_admin_alert() {
    let h = build_harness().await;
    let (member_id, mt_id) = seed_coterie_managed_member(&h.pool).await;
    seed_default_card(&h.saved_card_repo, member_id).await;

    // billing.max_retry_attempts defaults to 3 (migration 004), so a
    // row at retry_count = 2 tips terminal on the next failure
    // (2 + 1 >= 3).
    let scheduled_id =
        seed_scheduled_payment(&h.scheduled_repo, member_id, mt_id, 2).await;

    // Force the gateway to fail this charge.
    h.fake
        .next_payment_intent_err(AppError::External("card_declined".to_string()));

    h.billing
        .auto_renew
        .process_scheduled_payment(scheduled_id)
        .await
        .expect("process_scheduled_payment returns Ok even on terminal failure");

    assert_eq!(
        scheduled_status(&h.pool, scheduled_id).await,
        "failed",
        "row must transition to Failed on terminal retry"
    );

    let alerts = admin_alerts(&h.recorded_events);
    assert!(
        !alerts.is_empty(),
        "expected at least one AdminAlert on terminal failure, got none"
    );
    let terminal = alerts
        .iter()
        .find(|(s, _)| s.contains("Coterie-managed renewal failed (final)"))
        .expect("alert subject must contain 'Coterie-managed renewal failed (final)'");
    assert!(
        terminal.0.contains("Jane Smith"),
        "subject must include member name, got: {:?}",
        terminal.0,
    );
    assert!(
        terminal.1.contains("Jane Smith"),
        "body must include member name, got: {:?}",
        terminal.1,
    );
    assert!(
        terminal.1.contains("$50.00"),
        "body must include amount $50.00, got: {:?}",
        terminal.1,
    );
    assert!(
        terminal.1.contains("3 / 3"),
        "body must include retry count '3 / 3', got: {:?}",
        terminal.1,
    );
    assert!(
        terminal.1.contains("card_declined"),
        "body must include the last failure reason, got: {:?}",
        terminal.1,
    );
    assert!(
        terminal.1.contains(&format!("/portal/admin/members/{}", member_id)),
        "body must include the member-detail portal link, got: {:?}",
        terminal.1,
    );
}

#[tokio::test]
async fn transient_failure_does_not_dispatch_admin_alert() {
    let h = build_harness().await;
    let (member_id, mt_id) = seed_coterie_managed_member(&h.pool).await;
    seed_default_card(&h.saved_card_repo, member_id).await;

    // retry_count = 0; with max_retries = 3, a single failure here
    // increments to 1 and bounces the row back to Pending. No alert.
    let scheduled_id =
        seed_scheduled_payment(&h.scheduled_repo, member_id, mt_id, 0).await;

    h.fake
        .next_payment_intent_err(AppError::External("card_declined".to_string()));

    h.billing
        .auto_renew
        .process_scheduled_payment(scheduled_id)
        .await
        .expect("process_scheduled_payment returns Ok on transient failure");

    assert_eq!(
        scheduled_status(&h.pool, scheduled_id).await,
        "pending",
        "row must return to Pending for retry on a non-terminal failure"
    );

    let alerts = admin_alerts(&h.recorded_events);
    assert!(
        alerts.is_empty(),
        "no AdminAlert should be dispatched on a transient retry; got {:?}",
        alerts,
    );
}
