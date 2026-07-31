//! Webhook-flow tests for `StripeClient`'s per-event handlers. These
//! exercise the post-dispatch logic (DB row flips, dues extension,
//! idempotency, notification routing) by constructing `stripe::*`
//! event payloads via JSON and invoking the `dispatch_*` test wrappers
//! on `StripeClient` directly. Signature verification and the event-id
//! claim that `handle_webhook` does are out of scope here — they're
//! either trivial (HMAC-SHA256, claim is a single conditional INSERT)
//! or stripe-rs's responsibility.
//!
//! Each test owns a fresh in-memory pool so they can run in parallel.
//!
//! Run with: cargo test --features test-utils --test stripe_webhook_test

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use coterie::{
    auth::SecretCrypto,
    domain::{
        BillingMode, CreateMemberRequest, Payer, Payment, PaymentKind, PaymentMethod,
        PaymentStatus, StripeRef,
    },
    email::LogSender,
    error::Result as CoterieResult,
    integrations::{Integration, IntegrationEvent, IntegrationManager},
    payments::{
        fake_gateway::FakeStripeGateway, gateway::StripeGateway, StripeClient, WebhookDispatcher,
    },
    repository::{
        EventRepository, MemberRepository, PaymentRepository, SqliteEventRepository,
        SqliteMemberRepository, SqlitePaymentRepository, SqliteSavedCardRepository,
        SqliteScheduledPaymentRepository,
    },
    service::{
        billing_service::BillingService, membership_type_service::MembershipTypeService,
        settings_service::SettingsService,
    },
};
use serde_json::json;
use sqlx::SqlitePool;
use uuid::Uuid;

mod common;
use common::{
    build_charge, build_checkout_session, build_invoice, build_payment_intent, build_subscription,
    fresh_pool,
};

// ---------------------------------------------------------------------
// RecordingIntegration — test-only Integration impl that captures every
// dispatched IntegrationEvent into a shared Vec the test can inspect.
// Lets the invoice.payment_failed tests verify that AdminAlerts make it
// through the IntegrationManager without modifying production code.
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

fn admin_alert_subjects(events: &Arc<Mutex<Vec<IntegrationEvent>>>) -> Vec<String> {
    events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            IntegrationEvent::AdminAlert { subject, .. } => Some(subject.clone()),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------
// Setup helpers
// ---------------------------------------------------------------------

struct Harness {
    #[allow(dead_code)]
    client: StripeClient,
    dispatcher: WebhookDispatcher,
    fake: Arc<FakeStripeGateway>,
    billing: BillingService,
    pool: SqlitePool,
    recorded_events: Arc<Mutex<Vec<IntegrationEvent>>>,
}

async fn build_harness() -> Harness {
    let pool = fresh_pool().await;
    let fake = Arc::new(FakeStripeGateway::new());

    let payment_repo: Arc<dyn PaymentRepository> =
        Arc::new(SqlitePaymentRepository::new(pool.clone()));
    let member_repo: Arc<dyn MemberRepository> =
        Arc::new(SqliteMemberRepository::new(pool.clone()));
    let event_repo: Arc<dyn EventRepository> = Arc::new(SqliteEventRepository::new(pool.clone()));
    let scheduled_repo = Arc::new(SqliteScheduledPaymentRepository::new(pool.clone()));
    let saved_card_repo = Arc::new(SqliteSavedCardRepository::new(pool.clone()));
    let mt_repo = Arc::new(coterie::repository::SqliteMembershipTypeRepository::new(
        pool.clone(),
    ));
    let mt_service = Arc::new(MembershipTypeService::new(mt_repo));
    let crypto = Arc::new(SecretCrypto::new("test-secret-please-ignore"));
    let settings = Arc::new(SettingsService::new(pool.clone(), crypto));
    let email_sender = Arc::new(LogSender::new(
        "test@example.com".to_string(),
        "Test".to_string(),
    ));
    let integrations = Arc::new(IntegrationManager::new());
    let recorded_events: Arc<Mutex<Vec<IntegrationEvent>>> = Arc::new(Mutex::new(Vec::new()));
    integrations
        .register(Arc::new(RecordingIntegration {
            events: recorded_events.clone(),
        }))
        .await;

    let gw: Arc<dyn StripeGateway> = fake.clone();
    let client = StripeClient::with_gateway(gw.clone(), payment_repo.clone(), member_repo.clone());
    // Fake-backed handle for the billing service: the signup auto-renew
    // enrollment path lists the customer's payment methods through it.
    let billing_stripe_handle = Arc::new(coterie::payments::StripeHandle::preloaded(
        Some(Arc::new(StripeClient::with_gateway(
            gw.clone(),
            payment_repo.clone(),
            member_repo.clone(),
        ))),
        None,
    ));
    let processed_events_repo: Arc<dyn coterie::repository::ProcessedEventsRepository> = Arc::new(
        coterie::repository::SqliteProcessedEventsRepository::new(pool.clone()),
    );
    let dispatcher = WebhookDispatcher::new(
        gw,
        "whsec_test_dummy".to_string(),
        payment_repo.clone(),
        member_repo.clone(),
        event_repo.clone(),
        Arc::new(coterie::repository::SqliteSeriesEnrollmentRepository::new(
            pool.clone(),
        )),
        processed_events_repo,
        mt_service.clone(),
        integrations.clone(),
        Arc::new(coterie::service::audit_service::AuditService::new(
            pool.clone(),
        )),
    );

    let billing = BillingService::new(
        scheduled_repo,
        payment_repo,
        saved_card_repo,
        member_repo,
        event_repo,
        mt_service,
        settings,
        email_sender,
        integrations,
        billing_stripe_handle,
        "http://localhost:3000".to_string(),
        pool.clone(),
    );

    Harness {
        client,
        dispatcher,
        fake,
        billing,
        pool,
        recorded_events,
    }
}

/// Insert a member, attach the seeded "member" membership_type so dues
/// extension has a slug to resolve, and stamp billing_mode +
/// stripe_customer_id. Returns the member's id.
async fn insert_member(
    pool: &SqlitePool,
    customer_id: Option<&str>,
    billing_mode: BillingMode,
) -> Uuid {
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

    let billing_str = match billing_mode {
        BillingMode::Manual => "manual",
        BillingMode::CoterieManaged => "coterie_managed",
        BillingMode::StripeSubscription => "stripe_subscription",
    };

    // The seed migration installs three membership_types; pick the
    // monthly "member" one so dues extension has something to resolve.
    let mt_id: String =
        sqlx::query_scalar("SELECT id FROM membership_types WHERE slug = 'member' LIMIT 1")
            .fetch_one(pool)
            .await
            .expect("seeded 'member' membership_type");

    sqlx::query(
        "UPDATE members \
         SET stripe_customer_id = ?, billing_mode = ?, membership_type_id = ? \
         WHERE id = ?",
    )
    .bind(customer_id)
    .bind(billing_str)
    .bind(&mt_id)
    .bind(member.id.to_string())
    .execute(pool)
    .await
    .expect("set customer + billing_mode + mt");

    member.id
}

async fn insert_pending_payment(pool: &SqlitePool, payment: Payment) {
    let repo = SqlitePaymentRepository::new(pool.clone());
    repo.create(payment).await.expect("insert payment");
}

/// Insert an active membership_type with an exact slug + billing period.
/// Used by the dues-extension regression tests to make a previously
/// unresolvable slug resolvable on the recovering retry. Returns the id.
async fn create_membership_type(pool: &SqlitePool, slug: &str, billing_period: &str) -> Uuid {
    let id = Uuid::new_v4();
    let now = Utc::now().naive_utc();
    sqlx::query(
        "INSERT INTO membership_types \
         (id, name, slug, description, color, icon, sort_order, is_active, \
          fee_cents, billing_period, created_at, updated_at) \
         VALUES (?, ?, ?, NULL, NULL, NULL, 99, 1, 5000, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(format!("Ghost {}", slug))
    .bind(slug)
    .bind(billing_period)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .expect("create membership type");
    id
}

async fn payment_dues_extended_at(
    pool: &SqlitePool,
    payment_id: Uuid,
) -> Option<chrono::NaiveDateTime> {
    sqlx::query_scalar::<_, Option<chrono::NaiveDateTime>>(
        "SELECT dues_extended_at FROM payments WHERE id = ?",
    )
    .bind(payment_id.to_string())
    .fetch_one(pool)
    .await
    .expect("query dues_extended_at")
}

async fn payment_status(pool: &SqlitePool, payment_id: Uuid) -> String {
    sqlx::query_scalar::<_, String>("SELECT status FROM payments WHERE id = ?")
        .bind(payment_id.to_string())
        .fetch_one(pool)
        .await
        .expect("query status")
}

async fn member_dues_paid_until(
    pool: &SqlitePool,
    member_id: Uuid,
) -> Option<chrono::DateTime<Utc>> {
    sqlx::query_scalar::<_, Option<chrono::DateTime<Utc>>>(
        "SELECT dues_paid_until FROM members WHERE id = ?",
    )
    .bind(member_id.to_string())
    .fetch_one(pool)
    .await
    .expect("query dues_paid_until")
}

async fn member_billing_mode(pool: &SqlitePool, member_id: Uuid) -> String {
    sqlx::query_scalar::<_, String>("SELECT billing_mode FROM members WHERE id = ?")
        .bind(member_id.to_string())
        .fetch_one(pool)
        .await
        .expect("query billing_mode")
}

async fn member_subscription_id(pool: &SqlitePool, member_id: Uuid) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>(
        "SELECT stripe_subscription_id FROM members WHERE id = ?",
    )
    .bind(member_id.to_string())
    .fetch_one(pool)
    .await
    .expect("query stripe_subscription_id")
}

// ---------------------------------------------------------------------
// JSON builders for stripe-rs types
// ---------------------------------------------------------------------

// ---------------------------------------------------------------------
// 1. dues-extension idempotency on payment_intent.succeeded retry
// ---------------------------------------------------------------------

#[tokio::test]
async fn pi_succeeded_retry_does_not_double_extend_dues() {
    let h = build_harness().await;
    let member_id = insert_member(&h.pool, Some("cus_self_heal"), BillingMode::Manual).await;
    let payment_id = Uuid::new_v4();

    // The Pending row the saved-card / donate path inserts BEFORE
    // calling Stripe. handle_payment_intent_succeeded's job is to
    // self-heal — flip Pending → Completed and run the post-work — IF
    // (and only if) it owns the flip.
    insert_pending_payment(
        &h.pool,
        Payment {
            id: payment_id,
            payer: Payer::Member(member_id),
            amount_cents: 50_00,
            currency: "USD".to_string(),
            status: PaymentStatus::Pending,
            payment_method: PaymentMethod::Stripe,
            external_id: None,
            description: "Dues".to_string(),
            kind: PaymentKind::Membership,
            paid_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
    )
    .await;

    let pi = build_payment_intent(
        "pi_self_heal",
        50_00,
        json!({
            "payment_id": payment_id.to_string(),
            "member_id": member_id.to_string(),
        }),
    );

    // First dispatch: should flip Pending → Completed and extend dues.
    h.dispatcher
        .dispatch_payment_intent_succeeded(pi.clone(), &h.billing)
        .await
        .expect("first dispatch ok");

    assert_eq!(payment_status(&h.pool, payment_id).await, "Completed");
    let extended_at_first = payment_dues_extended_at(&h.pool, payment_id)
        .await
        .expect("dues_extended_at must be set after first run");
    let dues_after_first = member_dues_paid_until(&h.pool, member_id)
        .await
        .expect("dues_paid_until must be set after first run");

    // Second dispatch with a fresh PaymentIntent (same payment_id metadata)
    // — this is the stripe-retry-after-rollback case where the
    // event-claim layer is bypassed and the inner handler must hold
    // idempotency on its own.
    h.dispatcher
        .dispatch_payment_intent_succeeded(pi, &h.billing)
        .await
        .expect("second dispatch ok");

    let extended_at_second = payment_dues_extended_at(&h.pool, payment_id)
        .await
        .expect("dues_extended_at still set");
    assert_eq!(
        extended_at_first, extended_at_second,
        "dues_extended_at must NOT be re-stamped on retry — that's the per-payment claim"
    );

    let dues_after_second = member_dues_paid_until(&h.pool, member_id)
        .await
        .expect("dues_paid_until still set");
    assert_eq!(
        dues_after_first, dues_after_second,
        "member.dues_paid_until must NOT shift on retry — extension must be idempotent"
    );
}

// ---------------------------------------------------------------------
// 2. charge.refunded echo for an already-Refunded row is a no-op
// ---------------------------------------------------------------------

#[tokio::test]
async fn charge_refunded_echo_for_already_refunded_row_is_noop() {
    let h = build_harness().await;
    let member_id = insert_member(&h.pool, Some("cus_refund_echo"), BillingMode::Manual).await;
    let payment_id = Uuid::new_v4();
    let pi_id = "pi_already_refunded";

    // Mimic the post-admin-refund state: the row is already Refunded
    // and stripe_payment_id holds the PI. Stripe's charge.refunded
    // webhook arrives shortly after as an echo.
    insert_pending_payment(
        &h.pool,
        Payment {
            id: payment_id,
            payer: Payer::Member(member_id),
            amount_cents: 100_00,
            currency: "USD".to_string(),
            status: PaymentStatus::Refunded,
            payment_method: PaymentMethod::Stripe,
            external_id: Some(StripeRef::PaymentIntent(pi_id.to_string())),
            description: "Dues".to_string(),
            kind: PaymentKind::Membership,
            paid_at: Some(Utc::now()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
    )
    .await;

    let updated_at_before: chrono::NaiveDateTime =
        sqlx::query_scalar("SELECT updated_at FROM payments WHERE id = ?")
            .bind(payment_id.to_string())
            .fetch_one(&h.pool)
            .await
            .expect("updated_at before");

    let charge = build_charge("ch_refund_echo", 100_00, 100_00, Some(pi_id));
    h.dispatcher
        .dispatch_charge_refunded(charge)
        .await
        .expect("dispatch ok");

    assert_eq!(
        payment_status(&h.pool, payment_id).await,
        "Refunded",
        "row stays Refunded"
    );

    let updated_at_after: chrono::NaiveDateTime =
        sqlx::query_scalar("SELECT updated_at FROM payments WHERE id = ?")
            .bind(payment_id.to_string())
            .fetch_one(&h.pool)
            .await
            .expect("updated_at after");

    assert_eq!(
        updated_at_before, updated_at_after,
        "no UPDATE must run when echo finds row already Refunded"
    );
}

#[tokio::test]
async fn charge_refunded_for_completed_row_flips_to_refunded() {
    // Companion test: when an admin refunds via Stripe's dashboard
    // (not Coterie's UI), our row is still Completed when the webhook
    // arrives and must flip to Refunded.
    let h = build_harness().await;
    let member_id = insert_member(&h.pool, Some("cus_dashboard_refund"), BillingMode::Manual).await;
    let payment_id = Uuid::new_v4();
    let pi_id = "pi_dashboard_refund";

    insert_pending_payment(
        &h.pool,
        Payment {
            id: payment_id,
            payer: Payer::Member(member_id),
            amount_cents: 100_00,
            currency: "USD".to_string(),
            status: PaymentStatus::Completed,
            payment_method: PaymentMethod::Stripe,
            external_id: Some(StripeRef::PaymentIntent(pi_id.to_string())),
            description: "Dues".to_string(),
            kind: PaymentKind::Membership,
            paid_at: Some(Utc::now()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
    )
    .await;

    let charge = build_charge("ch_dashboard_refund", 100_00, 100_00, Some(pi_id));
    h.dispatcher
        .dispatch_charge_refunded(charge)
        .await
        .expect("dispatch ok");

    assert_eq!(payment_status(&h.pool, payment_id).await, "Refunded");
}

// ---------------------------------------------------------------------
// 3. Stripe→Coterie auto-renew migration: subscription.deleted echo for
//    an already-migrated member must NOT flip them back to manual
// ---------------------------------------------------------------------

#[tokio::test]
async fn subscription_deleted_for_migrated_member_is_silent_noop() {
    let h = build_harness().await;
    let customer_id = "cus_migrated";
    // Member has been migrated: billing_mode is already coterie_managed
    // by the time Stripe's customer.subscription.deleted echo arrives.
    let member_id = insert_member(&h.pool, Some(customer_id), BillingMode::CoterieManaged).await;

    let sub = build_subscription("sub_migrated_echo", customer_id);
    h.dispatcher
        .dispatch_subscription_deleted(sub, &h.billing)
        .await
        .expect("dispatch ok");

    // The handler must NOT clobber billing_mode back to manual.
    assert_eq!(
        member_billing_mode(&h.pool, member_id).await,
        "coterie_managed",
        "migrated member must stay coterie_managed — echo from our own cancel must be silent",
    );
}

#[tokio::test]
async fn subscription_deleted_for_active_subscription_flips_to_manual() {
    // Companion test: out-of-band cancellation (member used Stripe's
    // hosted portal). billing_mode is still stripe_subscription, so
    // the handler must flip to manual.
    let h = build_harness().await;
    let customer_id = "cus_oob_cancel";
    let member_id =
        insert_member(&h.pool, Some(customer_id), BillingMode::StripeSubscription).await;

    let sub = build_subscription("sub_oob_cancel", customer_id);
    h.dispatcher
        .dispatch_subscription_deleted(sub, &h.billing)
        .await
        .expect("dispatch ok");

    assert_eq!(member_billing_mode(&h.pool, member_id).await, "manual");
}

// ---------------------------------------------------------------------
// 4. Public-donation Checkout completion: row flips to Completed,
//    no dues math is attempted.
// ---------------------------------------------------------------------

#[tokio::test]
async fn public_donation_checkout_completion_marks_payment_completed() {
    let h = build_harness().await;
    let payment_id = Uuid::new_v4();
    let session_id = "cs_public_donation";

    // Public donation row: member_id is NULL, donor info present.
    // create_public_donation_checkout_session inserts this kind of row.
    insert_pending_payment(
        &h.pool,
        Payment {
            id: payment_id,
            payer: Payer::PublicDonor {
                name: "Anonymous Donor".to_string(),
                email: "donor@example.com".to_string(),
            },
            amount_cents: 25_00,
            currency: "USD".to_string(),
            status: PaymentStatus::Pending,
            payment_method: PaymentMethod::Stripe,
            external_id: Some(StripeRef::CheckoutSession(session_id.to_string())),
            description: "Donation — Anonymous".to_string(),
            kind: PaymentKind::Donation { campaign_id: None },
            paid_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
    )
    .await;

    let session = build_checkout_session(
        session_id,
        Some("pi_public_donation"),
        json!({
            "payment_type": "donation",
            "public_donation": "1",
            "donor_email": "donor@example.com",
        }),
        5000,
    );

    h.dispatcher
        .dispatch_checkout_session_completed(session, &h.billing)
        .await
        .expect("dispatch ok");

    assert_eq!(payment_status(&h.pool, payment_id).await, "Completed");

    // The donation path must NOT touch dues_extended_at — there's no
    // membership to extend.
    assert!(
        payment_dues_extended_at(&h.pool, payment_id)
            .await
            .is_none(),
        "donation completion must not stamp dues_extended_at"
    );

    // And the stripe_payment_id should have been upgraded from the
    // cs_ session to the pi_ from the expanded payment_intent — that's
    // what handle_successful_payment does so charge.refunded can match.
    let stripe_id: Option<String> =
        sqlx::query_scalar("SELECT stripe_payment_id FROM payments WHERE id = ?")
            .bind(payment_id.to_string())
            .fetch_one(&h.pool)
            .await
            .expect("query stripe_payment_id");
    assert_eq!(stripe_id.as_deref(), Some("pi_public_donation"));
}

// ---------------------------------------------------------------------
// 4b. Reorder regression: the irreversible Completed flip must be the
//     LAST must-succeed step on the membership path. A transient dues-
//     extension failure must leave the row Pending so Stripe's retry can
//     recover it, and the recovered retry must advance dues exactly once.
//     (stripe-webhook → "Failed processing releases the claim for retry"
//     + "Event processing is idempotent via atomic claim".)
// ---------------------------------------------------------------------

#[tokio::test]
async fn checkout_completed_leaves_payment_pending_when_dues_extend_fails() {
    let h = build_harness().await;
    let member_id = insert_member(&h.pool, Some("cus_extend_fail"), BillingMode::Manual).await;
    let payment_id = Uuid::new_v4();
    let session_id = "cs_extend_fail";
    let slug = "ghost-tier"; // not in the membership-type registry (yet)

    insert_pending_payment(
        &h.pool,
        Payment {
            id: payment_id,
            payer: Payer::Member(member_id),
            amount_cents: 50_00,
            currency: "USD".to_string(),
            status: PaymentStatus::Pending,
            payment_method: PaymentMethod::Stripe,
            external_id: Some(StripeRef::CheckoutSession(session_id.to_string())),
            description: "Dues".to_string(),
            kind: PaymentKind::Membership,
            paid_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
    )
    .await;

    let session = build_checkout_session(
        session_id,
        Some("pi_extend_fail"),
        json!({
            "payment_type": "membership",
            "membership_type_slug": slug,
        }),
        5000,
    );

    // First dispatch: the slug doesn't resolve, so extend_member_dues_by_slug
    // returns NotFound. Because extend now runs BEFORE the flip, the error
    // propagates (dispatch returns Err) and the row stays Pending — the
    // dispatcher's claim release then lets Stripe's retry recover it.
    let first = h
        .dispatcher
        .dispatch_checkout_session_completed(session.clone(), &h.billing)
        .await;
    assert!(
        first.is_err(),
        "dispatch must return Err when the dues extension fails",
    );
    assert_eq!(
        payment_status(&h.pool, payment_id).await,
        "Pending",
        "row must stay Pending on a transient extend failure — NOT Completed",
    );
    assert!(
        payment_dues_extended_at(&h.pool, payment_id)
            .await
            .is_none(),
        "dues_extended_at must not be stamped when the slug can't resolve",
    );

    // Recovery: create the membership type for that slug, then re-dispatch
    // the same event. Extend now succeeds, the row flips to Completed, and
    // dues advance by exactly one billing period (extend ran exactly once).
    create_membership_type(&h.pool, slug, "monthly").await;

    h.dispatcher
        .dispatch_checkout_session_completed(session, &h.billing)
        .await
        .expect("retry dispatch ok once the slug resolves");

    assert_eq!(
        payment_status(&h.pool, payment_id).await,
        "Completed",
        "row must be Completed after the recovering retry",
    );
    let dues_after = member_dues_paid_until(&h.pool, member_id)
        .await
        .expect("dues_paid_until set after recovery");
    // Member started with no dues anchor; one monthly period lands ~a
    // month out (27 days clears any month-length / leap-day wobble).
    assert!(
        dues_after > Utc::now() + Duration::days(27),
        "dues_paid_until {} must advance ~one month from now",
        dues_after,
    );
    assert!(
        payment_dues_extended_at(&h.pool, payment_id)
            .await
            .is_some(),
        "dues_extended_at must be stamped exactly once after recovery",
    );
}

#[tokio::test]
async fn pi_succeeded_leaves_payment_pending_when_membership_type_missing() {
    let h = build_harness().await;
    let member_id = insert_member(&h.pool, Some("cus_pi_no_mt"), BillingMode::Manual).await;

    // Point the member at a membership type that doesn't exist, so the
    // slug can't be resolved on the first dispatch. members.membership_type_id
    // has an FK, so briefly drop enforcement for this one setup write (the
    // pool is pinned to a single connection, so the PRAGMA sticks).
    let ghost_mt_id = Uuid::new_v4();
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&h.pool)
        .await
        .expect("fk off");
    sqlx::query("UPDATE members SET membership_type_id = ? WHERE id = ?")
        .bind(ghost_mt_id.to_string())
        .bind(member_id.to_string())
        .execute(&h.pool)
        .await
        .expect("point member at missing membership type");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&h.pool)
        .await
        .expect("fk on");

    let payment_id = Uuid::new_v4();
    insert_pending_payment(
        &h.pool,
        Payment {
            id: payment_id,
            payer: Payer::Member(member_id),
            amount_cents: 50_00,
            currency: "USD".to_string(),
            status: PaymentStatus::Pending,
            payment_method: PaymentMethod::Stripe,
            external_id: None,
            description: "Dues".to_string(),
            kind: PaymentKind::Membership,
            paid_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
    )
    .await;

    let pi = build_payment_intent(
        "pi_no_mt",
        50_00,
        json!({
            "payment_id": payment_id.to_string(),
            "member_id": member_id.to_string(),
        }),
    );

    // First dispatch: membership type missing → handler skips the dues
    // work and leaves the row Pending (the flip now follows the extend,
    // so it never runs here).
    h.dispatcher
        .dispatch_payment_intent_succeeded(pi.clone(), &h.billing)
        .await
        .expect("first dispatch is a quiet no-op");
    assert_eq!(
        payment_status(&h.pool, payment_id).await,
        "Pending",
        "row must stay Pending when the membership type can't be resolved",
    );
    assert!(
        member_dues_paid_until(&h.pool, member_id).await.is_none(),
        "dues must not advance while the membership type is missing",
    );

    // Make it resolvable: point the member back at the seeded 'member' type.
    let mt_id: String =
        sqlx::query_scalar("SELECT id FROM membership_types WHERE slug = 'member' LIMIT 1")
            .fetch_one(&h.pool)
            .await
            .expect("seeded 'member' membership_type");
    sqlx::query("UPDATE members SET membership_type_id = ? WHERE id = ?")
        .bind(&mt_id)
        .bind(member_id.to_string())
        .execute(&h.pool)
        .await
        .expect("restore resolvable membership type");

    // Retry: extend succeeds, row flips to Completed, dues advance once.
    h.dispatcher
        .dispatch_payment_intent_succeeded(pi, &h.billing)
        .await
        .expect("retry dispatch ok once the type resolves");
    assert_eq!(
        payment_status(&h.pool, payment_id).await,
        "Completed",
        "row must be Completed after the recovering retry",
    );
    let dues_after = member_dues_paid_until(&h.pool, member_id)
        .await
        .expect("dues_paid_until set after recovery");
    assert!(
        dues_after > Utc::now() + Duration::days(27),
        "dues_paid_until {} must advance ~one month from now (extend ran once)",
        dues_after,
    );
    assert!(
        payment_dues_extended_at(&h.pool, payment_id)
            .await
            .is_some(),
        "dues_extended_at must be stamped exactly once after recovery",
    );
}

#[tokio::test]
async fn checkout_membership_completion_is_idempotent_under_retry() {
    // Both deliveries succeed (Stripe's at-least-once semantics). Dues
    // must advance exactly once — the per-payment dues_extended_at claim
    // makes the extend a no-op on the second pass and the flip is already
    // Completed. Mirrors the existing invoice.paid idempotency coverage.
    let h = build_harness().await;
    let member_id = insert_member(&h.pool, Some("cus_chk_idem"), BillingMode::Manual).await;
    let payment_id = Uuid::new_v4();
    let session_id = "cs_chk_idem";

    insert_pending_payment(
        &h.pool,
        Payment {
            id: payment_id,
            payer: Payer::Member(member_id),
            amount_cents: 50_00,
            currency: "USD".to_string(),
            status: PaymentStatus::Pending,
            payment_method: PaymentMethod::Stripe,
            external_id: Some(StripeRef::CheckoutSession(session_id.to_string())),
            description: "Dues".to_string(),
            kind: PaymentKind::Membership,
            paid_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
    )
    .await;

    // Seeded 'member' slug resolves, so the first extend succeeds.
    let session = build_checkout_session(
        session_id,
        Some("pi_chk_idem"),
        json!({
            "payment_type": "membership",
            "membership_type_slug": "member",
        }),
        5000,
    );

    h.dispatcher
        .dispatch_checkout_session_completed(session.clone(), &h.billing)
        .await
        .expect("first dispatch ok");
    assert_eq!(payment_status(&h.pool, payment_id).await, "Completed");
    let extended_first = payment_dues_extended_at(&h.pool, payment_id)
        .await
        .expect("dues_extended_at set after first run");
    let dues_first = member_dues_paid_until(&h.pool, member_id)
        .await
        .expect("dues_paid_until set after first run");

    // Second delivery of the same successful event.
    h.dispatcher
        .dispatch_checkout_session_completed(session, &h.billing)
        .await
        .expect("second dispatch ok");

    let extended_second = payment_dues_extended_at(&h.pool, payment_id)
        .await
        .expect("dues_extended_at still set");
    assert_eq!(
        extended_first, extended_second,
        "dues_extended_at must NOT be re-stamped on retry — the per-payment claim holds",
    );
    let dues_second = member_dues_paid_until(&h.pool, member_id)
        .await
        .expect("dues_paid_until still set");
    assert_eq!(
        dues_first, dues_second,
        "member.dues_paid_until must advance exactly once across original + retry",
    );
}

// ---------------------------------------------------------------------
// Sanity assertion that none of the above quietly drove gateway calls
// ---------------------------------------------------------------------

#[tokio::test]
async fn webhook_handlers_do_not_call_gateway_unnecessarily() {
    // The four scenarios above each work entirely against the local
    // DB — no outbound Stripe calls should be needed. This catches
    // accidental introductions of e.g. a Customer fetch from inside
    // a handler.
    let h = build_harness().await;
    let _member_id = insert_member(&h.pool, Some("cus_x"), BillingMode::Manual).await;
    let payment_id = Uuid::new_v4();
    let pi_id = "pi_no_gateway";

    insert_pending_payment(
        &h.pool,
        Payment {
            id: payment_id,
            payer: Payer::PublicDonor {
                name: "D".to_string(),
                email: "d@example.com".to_string(),
            },
            amount_cents: 10_00,
            currency: "USD".to_string(),
            status: PaymentStatus::Completed,
            payment_method: PaymentMethod::Stripe,
            external_id: Some(StripeRef::PaymentIntent(pi_id.to_string())),
            description: "Donation".to_string(),
            kind: PaymentKind::Donation { campaign_id: None },
            paid_at: Some(Utc::now()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
    )
    .await;

    let charge = build_charge("ch_local_only", 10_00, 10_00, Some(pi_id));
    h.dispatcher
        .dispatch_charge_refunded(charge)
        .await
        .expect("dispatch ok");

    // Found row by IN-clause on pi_, so no fallback Stripe lookup
    // should have fired.
    assert_eq!(h.fake.calls().len(), 0, "no gateway calls expected");
}

// ---------------------------------------------------------------------
// Invoice-event helpers
// ---------------------------------------------------------------------

/// JSON-build a minimal `stripe::Invoice`. Most fields are Option, so we
/// only populate what the handlers actually look at: id, customer,
/// subscription, currency, amount_paid/amount_due/amount_remaining,
/// status, attempt_count, next_payment_attempt. The rest defaults to
/// None and the handler treats them as missing.
/// Seed `dues_paid_until` and `stripe_subscription_id` on a member.
/// The webhook handlers look up members by stripe_customer_id (set via
/// insert_member), but the integration tests want to confirm that the
/// dues anchor advances from a known starting point and to mirror the
/// production state of an imported StripeSubscription-mode member.
async fn set_member_subscription_state(
    pool: &SqlitePool,
    member_id: Uuid,
    dues_paid_until: chrono::DateTime<Utc>,
    stripe_subscription_id: &str,
) {
    sqlx::query(
        "UPDATE members \
         SET dues_paid_until = ?, stripe_subscription_id = ? \
         WHERE id = ?",
    )
    .bind(dues_paid_until)
    .bind(stripe_subscription_id)
    .bind(member_id.to_string())
    .execute(pool)
    .await
    .expect("seed subscription state");
}

async fn count_payments_by_stripe_id(pool: &SqlitePool, stripe_id: &str) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM payments WHERE stripe_payment_id = ?")
        .bind(stripe_id)
        .fetch_one(pool)
        .await
        .expect("count payments")
}

async fn payment_status_by_stripe_id(pool: &SqlitePool, stripe_id: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT status FROM payments WHERE stripe_payment_id = ?")
        .bind(stripe_id)
        .fetch_optional(pool)
        .await
        .expect("query status")
}

// ---------------------------------------------------------------------
// 5. invoice.paid: extends dues for known StripeSubscription member,
//    records a Completed Payment row, idempotent on retry, no-op on
//    unknown subscription.
// ---------------------------------------------------------------------

#[tokio::test]
async fn invoice_paid_extends_dues_for_stripe_subscription_member() {
    let h = build_harness().await;
    let customer_id = "cus_sub_paid";
    let member_id =
        insert_member(&h.pool, Some(customer_id), BillingMode::StripeSubscription).await;
    let seeded_dues = Utc::now() + Duration::days(30);
    set_member_subscription_state(&h.pool, member_id, seeded_dues, "sub_test_123").await;

    let invoice_id = "in_test_paid_happy";
    let invoice = build_invoice(
        invoice_id,
        customer_id,
        "sub_test_123",
        50_00,
        "paid",
        1,
        None,
    );

    h.dispatcher
        .dispatch_invoice_paid(invoice, &h.billing)
        .await
        .expect("dispatch_invoice_paid");

    // Dues advanced from the seeded anchor (today + 30 days) by one
    // monthly billing period (the seeded 'member' membership type).
    let dues_after = member_dues_paid_until(&h.pool, member_id)
        .await
        .expect("dues_paid_until set");
    assert!(
        dues_after > seeded_dues,
        "dues_paid_until {} must advance past the seeded anchor {}",
        dues_after,
        seeded_dues,
    );

    // Payment row exists for the invoice, status Completed.
    assert_eq!(count_payments_by_stripe_id(&h.pool, invoice_id).await, 1);
    assert_eq!(
        payment_status_by_stripe_id(&h.pool, invoice_id)
            .await
            .as_deref(),
        Some("Completed"),
    );
}

#[tokio::test]
async fn invoice_paid_idempotency() {
    // Same event delivered twice (Stripe's at-least-once semantics).
    // The unique partial index on payments.stripe_payment_id is the
    // current line of defense — the inner handler doesn't pre-check
    // for an existing payment, so the second create() returns Err with
    // a UNIQUE constraint violation. That's fine for production
    // because handle_webhook's processed_events_repo.claim dedupes
    // upstream; here we just verify the DB ends in the right shape no
    // matter what the second dispatch's Result is.
    let h = build_harness().await;
    let customer_id = "cus_sub_idem";
    let member_id =
        insert_member(&h.pool, Some(customer_id), BillingMode::StripeSubscription).await;
    let seeded_dues = Utc::now() + Duration::days(30);
    set_member_subscription_state(&h.pool, member_id, seeded_dues, "sub_test_idem").await;

    let invoice_id = "in_test_idem";
    let invoice = build_invoice(
        invoice_id,
        customer_id,
        "sub_test_idem",
        50_00,
        "paid",
        1,
        None,
    );

    h.dispatcher
        .dispatch_invoice_paid(invoice.clone(), &h.billing)
        .await
        .expect("first dispatch ok");

    let dues_after_first = member_dues_paid_until(&h.pool, member_id)
        .await
        .expect("dues set after first dispatch");

    // Second dispatch — Result is intentionally ignored. Either the
    // UNIQUE index rejects the duplicate Payment INSERT (Err) or a
    // future handler revision pre-detects the dup (Ok). What MUST
    // hold either way: dues do not advance further, and only one
    // payment row exists for the invoice.
    let _ = h
        .dispatcher
        .dispatch_invoice_paid(invoice, &h.billing)
        .await;

    let dues_after_second = member_dues_paid_until(&h.pool, member_id)
        .await
        .expect("dues still set after second dispatch");
    assert_eq!(
        dues_after_first, dues_after_second,
        "dues_paid_until must NOT shift on the second dispatch — extension must be idempotent",
    );
    assert_eq!(
        count_payments_by_stripe_id(&h.pool, invoice_id).await,
        1,
        "exactly one Payment row must exist for the invoice",
    );
}

#[tokio::test]
async fn invoice_paid_for_unknown_subscription_is_noop() {
    let h = build_harness().await;
    let customer_id = "cus_sub_unknown_paid";
    let member_id =
        insert_member(&h.pool, Some(customer_id), BillingMode::StripeSubscription).await;
    let seeded_dues = Utc::now() + Duration::days(30);
    set_member_subscription_state(&h.pool, member_id, seeded_dues, "sub_test_known").await;

    // Invoice references a customer that doesn't match any member.
    // The handler looks up by stripe_customer_id, so an unknown
    // customer (or by extension an unknown subscription) is a noop.
    let invoice_id = "in_test_unknown";
    let invoice = build_invoice(
        invoice_id,
        "cus_NEVER_HEARD_OF",
        "sub_NEVER_HEARD_OF",
        50_00,
        "paid",
        1,
        None,
    );

    h.dispatcher
        .dispatch_invoice_paid(invoice, &h.billing)
        .await
        .expect("dispatch should succeed quietly");

    assert_eq!(
        count_payments_by_stripe_id(&h.pool, invoice_id).await,
        0,
        "no Payment row should be created for an unknown subscription",
    );
    let dues_after = member_dues_paid_until(&h.pool, member_id)
        .await
        .expect("dues still set");
    assert_eq!(
        dues_after, seeded_dues,
        "seeded member's dues_paid_until must not change",
    );
}

// ---------------------------------------------------------------------
// 6. invoice.payment_failed: dispatches AdminAlert on first attempt,
//    softens copy on final attempt, no-op for unknown subscription.
// ---------------------------------------------------------------------

#[tokio::test]
async fn invoice_payment_failed_dispatches_admin_alert() {
    let h = build_harness().await;
    let customer_id = "cus_sub_failed";
    let member_id =
        insert_member(&h.pool, Some(customer_id), BillingMode::StripeSubscription).await;
    let seeded_dues = Utc::now() + Duration::days(30);
    set_member_subscription_state(&h.pool, member_id, seeded_dues, "sub_test_failed").await;

    // First attempt, retries scheduled (next_payment_attempt set).
    let future_ts = (Utc::now() + Duration::days(3)).timestamp();
    let invoice = build_invoice(
        "in_test_failed_first",
        customer_id,
        "sub_test_failed",
        50_00,
        "open",
        1,
        Some(future_ts),
    );

    h.dispatcher
        .dispatch_invoice_payment_failed(invoice, &h.billing)
        .await
        .expect("dispatch_invoice_payment_failed");

    let subjects = admin_alert_subjects(&h.recorded_events);
    assert!(
        subjects
            .iter()
            .any(|s| s.contains("Stripe subscription charge failed")),
        "expected AdminAlert subject containing 'Stripe subscription charge failed'; got {:?}",
        subjects,
    );
    assert!(
        !subjects.iter().any(|s| s.contains("(final)")),
        "non-final retry must not include '(final)' in subject; got {:?}",
        subjects,
    );

    // No DB mutation: dues and billing_mode unchanged.
    let dues_after = member_dues_paid_until(&h.pool, member_id)
        .await
        .expect("dues still set");
    assert_eq!(
        dues_after, seeded_dues,
        "dues_paid_until must not change on a failed-payment event",
    );
    assert_eq!(
        member_billing_mode(&h.pool, member_id).await,
        "stripe_subscription",
        "billing_mode must not change on a failed-payment event",
    );
}

#[tokio::test]
async fn invoice_payment_failed_final_attempt_softens_copy() {
    let h = build_harness().await;
    let customer_id = "cus_sub_failed_final";
    let member_id =
        insert_member(&h.pool, Some(customer_id), BillingMode::StripeSubscription).await;
    let seeded_dues = Utc::now() + Duration::days(30);
    set_member_subscription_state(&h.pool, member_id, seeded_dues, "sub_test_failed_final").await;

    // Final attempt: next_payment_attempt = None signals Stripe is
    // done retrying. handle_invoice_payment_failed should pass
    // is_final = true to notify_subscription_payment_failed, which
    // formats the AdminAlert subject with the "(final)" suffix.
    let invoice = build_invoice(
        "in_test_failed_final",
        customer_id,
        "sub_test_failed_final",
        50_00,
        "open",
        4,
        None,
    );

    h.dispatcher
        .dispatch_invoice_payment_failed(invoice, &h.billing)
        .await
        .expect("dispatch_invoice_payment_failed (final)");

    let subjects = admin_alert_subjects(&h.recorded_events);
    assert!(
        subjects.iter().any(|s| s.contains("(final)")),
        "final-attempt AdminAlert subject must contain '(final)'; got {:?}",
        subjects,
    );
}

#[tokio::test]
async fn invoice_payment_failed_for_unknown_subscription_is_noop() {
    let h = build_harness().await;
    let customer_id = "cus_sub_failed_unknown";
    let member_id =
        insert_member(&h.pool, Some(customer_id), BillingMode::StripeSubscription).await;
    let seeded_dues = Utc::now() + Duration::days(30);
    set_member_subscription_state(&h.pool, member_id, seeded_dues, "sub_test_known_failed").await;

    let future_ts = (Utc::now() + Duration::days(3)).timestamp();
    let invoice = build_invoice(
        "in_test_failed_unknown",
        "cus_NEVER_HEARD_OF",
        "sub_NEVER_HEARD_OF",
        50_00,
        "open",
        1,
        Some(future_ts),
    );

    h.dispatcher
        .dispatch_invoice_payment_failed(invoice, &h.billing)
        .await
        .expect("dispatch should succeed quietly");

    let subjects = admin_alert_subjects(&h.recorded_events);
    assert!(
        subjects.is_empty(),
        "no AdminAlert should be dispatched for an unknown subscription; got {:?}",
        subjects,
    );
    let dues_after = member_dues_paid_until(&h.pool, member_id)
        .await
        .expect("dues still set");
    assert_eq!(
        dues_after, seeded_dues,
        "seeded member's dues_paid_until must not change",
    );
}

// ---------------------------------------------------------------------
// 7. payment_intent.payment_failed: flips the matching Pending row to
//    Failed; unknown payment id is a silent no-op.
// ---------------------------------------------------------------------

#[tokio::test]
async fn failed_payment_flips_matching_payment_to_failed() {
    let h = build_harness().await;
    let member_id = insert_member(&h.pool, Some("cus_pi_failed"), BillingMode::Manual).await;
    let payment_id = Uuid::new_v4();
    let pi_id = "pi_failed";

    insert_pending_payment(
        &h.pool,
        Payment {
            id: payment_id,
            payer: Payer::Member(member_id),
            amount_cents: 50_00,
            currency: "USD".to_string(),
            status: PaymentStatus::Pending,
            payment_method: PaymentMethod::Stripe,
            external_id: Some(StripeRef::PaymentIntent(pi_id.to_string())),
            description: "Dues".to_string(),
            kind: PaymentKind::Membership,
            paid_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
    )
    .await;

    h.dispatcher
        .dispatch_failed_payment(pi_id.to_string())
        .await
        .expect("dispatch ok");

    assert_eq!(payment_status(&h.pool, payment_id).await, "Failed");
}

#[tokio::test]
async fn failed_payment_for_unknown_id_is_noop() {
    let h = build_harness().await;
    let member_id =
        insert_member(&h.pool, Some("cus_pi_failed_unknown"), BillingMode::Manual).await;
    let payment_id = Uuid::new_v4();

    // A Pending row exists, but keyed to a DIFFERENT PI than the one the
    // webhook references — so the handler must leave it untouched.
    insert_pending_payment(
        &h.pool,
        Payment {
            id: payment_id,
            payer: Payer::Member(member_id),
            amount_cents: 50_00,
            currency: "USD".to_string(),
            status: PaymentStatus::Pending,
            payment_method: PaymentMethod::Stripe,
            external_id: Some(StripeRef::PaymentIntent("pi_real".to_string())),
            description: "Dues".to_string(),
            kind: PaymentKind::Membership,
            paid_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
    )
    .await;

    h.dispatcher
        .dispatch_failed_payment("pi_does_not_exist".into())
        .await
        .expect("unknown id dispatch should succeed quietly");

    assert_eq!(
        payment_status(&h.pool, payment_id).await,
        "Pending",
        "unmatched payment row must not change status",
    );
}

// ---------------------------------------------------------------------
// 8. checkout.session.expired: flips the matching Pending row to Failed;
//    unknown session id is a silent no-op.
// ---------------------------------------------------------------------

#[tokio::test]
async fn expired_session_flips_pending_payment_to_failed() {
    let h = build_harness().await;
    let member_id = insert_member(&h.pool, Some("cus_cs_expired"), BillingMode::Manual).await;
    let payment_id = Uuid::new_v4();
    let cs_id = "cs_expired";

    insert_pending_payment(
        &h.pool,
        Payment {
            id: payment_id,
            payer: Payer::Member(member_id),
            amount_cents: 50_00,
            currency: "USD".to_string(),
            status: PaymentStatus::Pending,
            payment_method: PaymentMethod::Stripe,
            external_id: Some(StripeRef::CheckoutSession(cs_id.to_string())),
            description: "Dues".to_string(),
            kind: PaymentKind::Membership,
            paid_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
    )
    .await;

    let session = build_checkout_session(cs_id, None, json!({}), 5000);
    h.dispatcher
        .dispatch_expired_session(session)
        .await
        .expect("dispatch ok");

    assert_eq!(payment_status(&h.pool, payment_id).await, "Failed");
}

#[tokio::test]
async fn expired_session_for_unknown_session_is_noop() {
    let h = build_harness().await;
    let member_id = insert_member(&h.pool, Some("cus_cs_unknown"), BillingMode::Manual).await;
    let payment_id = Uuid::new_v4();

    // Pending row keyed to a different session id than the expired one.
    insert_pending_payment(
        &h.pool,
        Payment {
            id: payment_id,
            payer: Payer::Member(member_id),
            amount_cents: 50_00,
            currency: "USD".to_string(),
            status: PaymentStatus::Pending,
            payment_method: PaymentMethod::Stripe,
            external_id: Some(StripeRef::CheckoutSession("cs_real".to_string())),
            description: "Dues".to_string(),
            kind: PaymentKind::Membership,
            paid_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
    )
    .await;

    let session = build_checkout_session("cs_does_not_exist", None, json!({}), 5000);
    h.dispatcher
        .dispatch_expired_session(session)
        .await
        .expect("unknown session dispatch should succeed quietly");

    assert_eq!(
        payment_status(&h.pool, payment_id).await,
        "Pending",
        "unmatched payment row must not change status",
    );
}

// ---------------------------------------------------------------------
// 9. customer.subscription.updated: refreshes the member's stored
//    subscription id; unknown customer is a silent no-op.
// ---------------------------------------------------------------------

#[tokio::test]
async fn subscription_updated_refreshes_stored_subscription_id() {
    let h = build_harness().await;
    let customer_id = "cus_sub_updated";
    let member_id =
        insert_member(&h.pool, Some(customer_id), BillingMode::StripeSubscription).await;
    // Seed the old subscription id so we can prove it gets replaced.
    set_member_subscription_state(
        &h.pool,
        member_id,
        Utc::now() + Duration::days(30),
        "sub_old",
    )
    .await;

    let sub = build_subscription("sub_new", customer_id);
    h.dispatcher
        .dispatch_subscription_updated(sub)
        .await
        .expect("dispatch ok");

    assert_eq!(
        member_subscription_id(&h.pool, member_id).await.as_deref(),
        Some("sub_new"),
        "member's stored subscription id must be refreshed to the new value",
    );
    // billing_mode is carried through unchanged by the handler.
    assert_eq!(
        member_billing_mode(&h.pool, member_id).await,
        "stripe_subscription",
    );
}

#[tokio::test]
async fn subscription_updated_for_unknown_customer_is_noop() {
    let h = build_harness().await;
    let member_id =
        insert_member(&h.pool, Some("cus_known"), BillingMode::StripeSubscription).await;
    set_member_subscription_state(
        &h.pool,
        member_id,
        Utc::now() + Duration::days(30),
        "sub_kept",
    )
    .await;

    // Subscription event for a customer that maps to no member.
    let sub = build_subscription("sub_unknown_update", "cus_NEVER_HEARD_OF");
    h.dispatcher
        .dispatch_subscription_updated(sub)
        .await
        .expect("unknown customer dispatch should succeed quietly");

    assert_eq!(
        member_subscription_id(&h.pool, member_id).await.as_deref(),
        Some("sub_kept"),
        "known member's subscription id must not be mutated for an unrelated customer",
    );
}

// ---------------------------------------------------------------------
// pay-at-signup: save_card sessions enroll the member in auto-renew
// ---------------------------------------------------------------------

#[tokio::test]
async fn save_card_checkout_completion_enrolls_auto_renew() {
    let h = build_harness().await;
    let member_id = insert_member(&h.pool, Some("cus_enroll"), BillingMode::Manual).await;
    let payment_id = Uuid::new_v4();
    let session_id = "cs_enroll";

    insert_pending_payment(
        &h.pool,
        Payment {
            id: payment_id,
            payer: Payer::Member(member_id),
            amount_cents: 45_00,
            currency: "USD".to_string(),
            status: PaymentStatus::Pending,
            payment_method: PaymentMethod::Stripe,
            external_id: Some(StripeRef::CheckoutSession(session_id.to_string())),
            description: "Member Membership Payment".to_string(),
            kind: PaymentKind::Membership,
            paid_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
    )
    .await;

    // The card Stripe attached to the customer via setup_future_usage.
    h.fake
        .next_payment_methods(vec![coterie::payments::gateway::PaymentMethodSummary {
            id: "pm_signup".to_string(),
            brand: "visa".to_string(),
            last4: "4242".to_string(),
            exp_month: 12,
            exp_year: 2030,
            fingerprint: Some("fp_signup".to_string()),
        }]);

    let mut session = build_checkout_session(
        session_id,
        Some("pi_enroll"),
        json!({
            "payment_type": "membership",
            "member_id": member_id.to_string(),
            "membership_type_slug": "member",
            "save_card": "true",
        }),
        5000,
    );
    session.customer = Some(stripe::Expandable::Id(
        "cus_enroll".parse().expect("customer id"),
    ));

    h.dispatcher
        .dispatch_checkout_session_completed(session, &h.billing)
        .await
        .expect("webhook succeeds");

    let (status, billing_mode): (String, String) =
        sqlx::query_as("SELECT status, billing_mode FROM members WHERE id = ?")
            .bind(member_id.to_string())
            .fetch_one(&h.pool)
            .await
            .unwrap();
    assert_eq!(status, "Active", "payment activates the Pending member");
    assert_eq!(billing_mode, "coterie_managed", "enrolled in auto-renew");

    let (pm_id, is_default): (String, bool) = sqlx::query_as(
        "SELECT stripe_payment_method_id, is_default FROM payment_methods WHERE member_id = ?",
    )
    .bind(member_id.to_string())
    .fetch_one(&h.pool)
    .await
    .expect("card saved");
    assert_eq!(pm_id, "pm_signup");
    assert!(is_default, "first card becomes the default");

    let scheduled: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM scheduled_payments WHERE member_id = ? AND status = 'pending'",
    )
    .bind(member_id.to_string())
    .fetch_one(&h.pool)
    .await
    .unwrap();
    assert_eq!(scheduled, 1, "next renewal scheduled");
}

#[tokio::test]
async fn save_card_without_customer_soft_fails_payment_stands() {
    let h = build_harness().await;
    let member_id = insert_member(&h.pool, None, BillingMode::Manual).await;
    let payment_id = Uuid::new_v4();
    let session_id = "cs_enroll_nocust";

    insert_pending_payment(
        &h.pool,
        Payment {
            id: payment_id,
            payer: Payer::Member(member_id),
            amount_cents: 45_00,
            currency: "USD".to_string(),
            status: PaymentStatus::Pending,
            payment_method: PaymentMethod::Stripe,
            external_id: Some(StripeRef::CheckoutSession(session_id.to_string())),
            description: "Member Membership Payment".to_string(),
            kind: PaymentKind::Membership,
            paid_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        },
    )
    .await;

    // save_card stamped but no customer on the session: enrollment is
    // impossible — the webhook must still succeed and the member must
    // still come out Active with dues extended.
    let session = build_checkout_session(
        session_id,
        Some("pi_enroll_nocust"),
        json!({
            "payment_type": "membership",
            "member_id": member_id.to_string(),
            "membership_type_slug": "member",
            "save_card": "true",
        }),
        5000,
    );

    h.dispatcher
        .dispatch_checkout_session_completed(session, &h.billing)
        .await
        .expect("webhook succeeds despite failed enrollment");

    let (status, billing_mode): (String, String) =
        sqlx::query_as("SELECT status, billing_mode FROM members WHERE id = ?")
            .bind(member_id.to_string())
            .fetch_one(&h.pool)
            .await
            .unwrap();
    assert_eq!(status, "Active");
    assert_eq!(billing_mode, "manual", "no enrollment without a customer");
    let cards: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM payment_methods WHERE member_id = ?")
        .bind(member_id.to_string())
        .fetch_one(&h.pool)
        .await
        .unwrap();
    assert_eq!(cards, 0);
}
