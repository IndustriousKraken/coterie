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

use std::sync::Arc;

use coterie::{
    auth::SecretCrypto,
    domain::{
        BillingMode, CreateMemberRequest, MembershipType, Payer, Payment, PaymentKind,
        PaymentMethod, PaymentStatus, StripeRef,
    },
    email::LogSender,
    integrations::IntegrationManager,
    payments::{
        fake_gateway::FakeStripeGateway, gateway::StripeGateway, StripeClient, WebhookDispatcher,
    },
    repository::{
        MemberRepository, PaymentRepository, SqliteMemberRepository, SqlitePaymentRepository,
        SqliteSavedCardRepository, SqliteScheduledPaymentRepository,
    },
    service::{
        billing_service::BillingService, membership_type_service::MembershipTypeService,
        settings_service::SettingsService,
    },
};
use chrono::Utc;
use serde_json::json;
use sqlx::{Executor, SqlitePool};
use uuid::Uuid;

// ---------------------------------------------------------------------
// Setup helpers
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
    #[allow(dead_code)]
    client: StripeClient,
    dispatcher: WebhookDispatcher,
    fake: Arc<FakeStripeGateway>,
    billing: BillingService,
    pool: SqlitePool,
}

async fn build_harness() -> Harness {
    let pool = fresh_pool().await;
    let fake = Arc::new(FakeStripeGateway::new());

    let payment_repo: Arc<dyn PaymentRepository> =
        Arc::new(SqlitePaymentRepository::new(pool.clone()));
    let member_repo: Arc<dyn MemberRepository> =
        Arc::new(SqliteMemberRepository::new(pool.clone()));
    let scheduled_repo = Arc::new(SqliteScheduledPaymentRepository::new(pool.clone()));
    let saved_card_repo = Arc::new(SqliteSavedCardRepository::new(pool.clone()));
    let mt_repo = Arc::new(coterie::repository::SqliteMembershipTypeRepository::new(pool.clone()));
    let mt_service = Arc::new(MembershipTypeService::new(mt_repo));
    let crypto = Arc::new(SecretCrypto::new("test-secret-please-ignore"));
    let settings = Arc::new(SettingsService::new(pool.clone(), crypto));
    let email_sender = Arc::new(LogSender::new(
        "test@example.com".to_string(),
        "Test".to_string(),
    ));
    let integrations = Arc::new(IntegrationManager::new());

    let gw: Arc<dyn StripeGateway> = fake.clone();
    let client = StripeClient::with_gateway(
        gw.clone(),
        payment_repo.clone(),
        member_repo.clone(),
    );
    let dispatcher = WebhookDispatcher::new(
        gw,
        "whsec_test_dummy".to_string(),
        payment_repo.clone(),
        member_repo.clone(),
        mt_service.clone(),
        integrations.clone(),
        pool.clone(),
    );

    let billing = BillingService::new(
        scheduled_repo,
        payment_repo,
        saved_card_repo,
        member_repo,
        mt_service,
        settings,
        email_sender,
        integrations,
        None, // stripe_client — none of our tests invoke billing paths that need it
        "http://localhost:3000".to_string(),
        pool.clone(),
    );

    Harness { client, dispatcher, fake, billing, pool }
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
            membership_type: MembershipType::Regular,
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

async fn payment_dues_extended_at(pool: &SqlitePool, payment_id: Uuid) -> Option<chrono::NaiveDateTime> {
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

// ---------------------------------------------------------------------
// JSON builders for stripe-rs types
// ---------------------------------------------------------------------

fn build_payment_intent(
    id: &str,
    amount: i64,
    metadata: serde_json::Value,
) -> stripe::PaymentIntent {
    let body = json!({
        "id": id,
        "object": "payment_intent",
        "amount": amount,
        "amount_received": amount,
        "amount_capturable": 0,
        "currency": "usd",
        "status": "succeeded",
        "livemode": false,
        "created": Utc::now().timestamp(),
        "metadata": metadata,
        "capture_method": "automatic",
        "confirmation_method": "automatic",
        "payment_method_types": ["card"],
    });
    serde_json::from_value(body).expect("PaymentIntent from JSON")
}

fn build_charge(
    id: &str,
    amount: i64,
    amount_refunded: i64,
    payment_intent: Option<&str>,
) -> stripe::Charge {
    let body = json!({
        "id": id,
        "object": "charge",
        "amount": amount,
        "amount_captured": amount,
        "amount_refunded": amount_refunded,
        "billing_details": {
            "address": null,
            "email": null,
            "name": null,
            "phone": null,
        },
        "currency": "usd",
        "captured": true,
        "created": Utc::now().timestamp(),
        "disputed": false,
        "livemode": false,
        "paid": true,
        "refunded": amount_refunded >= amount,
        "status": "succeeded",
        "payment_intent": payment_intent,
        "metadata": {},
    });
    serde_json::from_value(body).expect("Charge from JSON")
}

fn build_subscription(id: &str, customer_id: &str) -> stripe::Subscription {
    let body = json!({
        "id": id,
        "object": "subscription",
        "customer": customer_id,
        "status": "canceled",
        "created": Utc::now().timestamp(),
        "current_period_start": Utc::now().timestamp(),
        "current_period_end": Utc::now().timestamp() + 86400,
        "start_date": Utc::now().timestamp(),
        "livemode": false,
        "cancel_at_period_end": false,
        "collection_method": "charge_automatically",
        "automatic_tax": { "enabled": false, "liability": null },
        "billing_cycle_anchor": Utc::now().timestamp(),
        "currency": "usd",
        "metadata": {},
        "items": {
            "object": "list",
            "data": [],
            "has_more": false,
            "total_count": 0,
            "url": "/v1/subscription_items"
        },
    });
    serde_json::from_value(body).expect("Subscription from JSON")
}

fn build_checkout_session(
    id: &str,
    payment_intent_id: Option<&str>,
    metadata: serde_json::Value,
) -> stripe::CheckoutSession {
    let body = json!({
        "id": id,
        "object": "checkout.session",
        "livemode": false,
        "mode": "payment",
        "status": "complete",
        "payment_status": "paid",
        "created": Utc::now().timestamp(),
        "expires_at": Utc::now().timestamp() + 86400,
        "currency": "usd",
        "amount_total": 5000,
        "amount_subtotal": 5000,
        "metadata": metadata,
        "payment_intent": payment_intent_id,
        "automatic_tax": { "enabled": false, "liability": null, "status": null },
        "custom_fields": [],
        "custom_text": {
            "after_submit": null,
            "shipping_address": null,
            "submit": null,
            "terms_of_service_acceptance": null
        },
        "payment_method_types": ["card"],
        "shipping_options": [],
    });
    serde_json::from_value(body).expect("CheckoutSession from JSON")
}

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
    let extended_at_first = payment_dues_extended_at(&h.pool, payment_id).await
        .expect("dues_extended_at must be set after first run");
    let dues_after_first = member_dues_paid_until(&h.pool, member_id).await
        .expect("dues_paid_until must be set after first run");

    // Second dispatch with a fresh PaymentIntent (same payment_id metadata)
    // — this is the stripe-retry-after-rollback case where the
    // event-claim layer is bypassed and the inner handler must hold
    // idempotency on its own.
    h.dispatcher
        .dispatch_payment_intent_succeeded(pi, &h.billing)
        .await
        .expect("second dispatch ok");

    let extended_at_second = payment_dues_extended_at(&h.pool, payment_id).await
        .expect("dues_extended_at still set");
    assert_eq!(
        extended_at_first, extended_at_second,
        "dues_extended_at must NOT be re-stamped on retry — that's the per-payment claim"
    );

    let dues_after_second = member_dues_paid_until(&h.pool, member_id).await
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

    let updated_at_before: chrono::NaiveDateTime = sqlx::query_scalar(
        "SELECT updated_at FROM payments WHERE id = ?",
    )
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

    let updated_at_after: chrono::NaiveDateTime = sqlx::query_scalar(
        "SELECT updated_at FROM payments WHERE id = ?",
    )
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
    );

    h.dispatcher
        .dispatch_checkout_session_completed(session, &h.billing)
        .await
        .expect("dispatch ok");

    assert_eq!(payment_status(&h.pool, payment_id).await, "Completed");

    // The donation path must NOT touch dues_extended_at — there's no
    // membership to extend.
    assert!(
        payment_dues_extended_at(&h.pool, payment_id).await.is_none(),
        "donation completion must not stamp dues_extended_at"
    );

    // And the stripe_payment_id should have been upgraded from the
    // cs_ session to the pi_ from the expanded payment_intent — that's
    // what handle_successful_payment does so charge.refunded can match.
    let stripe_id: Option<String> = sqlx::query_scalar(
        "SELECT stripe_payment_id FROM payments WHERE id = ?",
    )
    .bind(payment_id.to_string())
    .fetch_one(&h.pool)
    .await
    .expect("query stripe_payment_id");
    assert_eq!(stripe_id.as_deref(), Some("pi_public_donation"));
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
