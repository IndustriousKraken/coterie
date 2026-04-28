//! Integration tests for `StripeClient` methods that go through the
//! `StripeGateway` trait. These exercise the application-level logic
//! (DB lookups, validation, error mapping) with a controllable fake in
//! place of real Stripe I/O.
//!
//! Run with:  cargo test --features test-utils --test stripe_gateway_test
//! (the `test-utils` feature exposes `FakeStripeGateway`)
//!
//! Each test gets its own in-memory SQLite pool and full migration set,
//! so they're hermetic and runnable in parallel without coordination.

use std::collections::HashMap;
use std::sync::Arc;

use coterie::{
    domain::{CreateMemberRequest, MembershipType},
    error::AppError,
    integrations::IntegrationManager,
    payments::{
        fake_gateway::{FakeCall, FakeStripeGateway},
        gateway::{
            PaymentIntentResult, RefundOutput, RetrievedCheckoutSession,
            RetrievedInvoice, StripeGateway,
        },
        StripeClient,
    },
    repository::{
        DonationCampaignRepository, MemberRepository, PaymentRepository,
        SqliteDonationCampaignRepository, SqliteMemberRepository,
        SqlitePaymentRepository,
    },
    service::membership_type_service::MembershipTypeService,
};
use sqlx::{Executor, SqlitePool};
use uuid::Uuid;

/// Spin up a fresh in-memory DB, run every migration, and return a
/// pool ready for service construction.
async fn fresh_pool() -> SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1) // :memory: is connection-private, so single-connection only
        .after_connect(|conn, _| Box::pin(async move {
            conn.execute("PRAGMA foreign_keys = ON").await?;
            Ok(())
        }))
        .connect("sqlite::memory:")
        .await
        .expect("connect to :memory:");

    sqlx::migrate!("./migrations").run(&pool).await
        .expect("run migrations");

    pool
}

/// Build a `StripeClient` wired to a `FakeStripeGateway` — returns
/// both so tests can drive the fake (queue responses, inspect calls)
/// while exercising the client.
fn build_client_with_fake(
    pool: SqlitePool,
) -> (StripeClient, Arc<FakeStripeGateway>) {
    let fake = Arc::new(FakeStripeGateway::new());
    let payment_repo: Arc<dyn PaymentRepository> =
        Arc::new(SqlitePaymentRepository::new(pool.clone()));
    let member_repo: Arc<dyn MemberRepository> =
        Arc::new(SqliteMemberRepository::new(pool.clone()));
    let _campaign_repo: Arc<dyn DonationCampaignRepository> =
        Arc::new(SqliteDonationCampaignRepository::new(pool.clone()));
    let mt_repo = Arc::new(coterie::repository::SqliteMembershipTypeRepository::new(pool.clone()));
    let mt_service = Arc::new(MembershipTypeService::new(mt_repo));
    let integration_manager = Arc::new(IntegrationManager::new());

    // The gateway is the fake; every StripeClient method routes through
    // it now, so there's no longer a stripe-rs Client to wire up.
    let gw: Arc<dyn StripeGateway> = fake.clone();
    let client = StripeClient::with_gateway(
        gw,
        "whsec_test_dummy".to_string(),
        payment_repo,
        member_repo,
        mt_service,
        integration_manager,
        pool,
    );
    (client, fake)
}

/// Minimal helper: insert a Member with a stripe_customer_id set, so
/// `charge_saved_card` can find it. Returns the member's ID.
async fn insert_member_with_customer(
    pool: &SqlitePool,
    customer_id: &str,
) -> Uuid {
    let repo = SqliteMemberRepository::new(pool.clone());
    let member = repo.create(CreateMemberRequest {
        email: format!("test-{}@example.com", Uuid::new_v4()),
        username: format!("test_{}", Uuid::new_v4().simple()),
        full_name: "Test Member".to_string(),
        password: "p4ssword_long_enough".to_string(),
        membership_type: MembershipType::Regular,
    }).await.expect("create member");

    sqlx::query("UPDATE members SET stripe_customer_id = ? WHERE id = ?")
        .bind(customer_id)
        .bind(member.id.to_string())
        .execute(pool)
        .await
        .expect("attach stripe_customer_id");
    member.id
}

// ---------------------------------------------------------------------
// charge_saved_card
// ---------------------------------------------------------------------

#[tokio::test]
async fn charge_saved_card_happy_path() {
    let pool = fresh_pool().await;
    let member_id = insert_member_with_customer(&pool, "cus_known").await;
    let (client, fake) = build_client_with_fake(pool);

    fake.next_payment_intent(PaymentIntentResult::Succeeded {
        id: "pi_known_charge".to_string(),
    });

    let payment_id = Uuid::new_v4();
    let result = client.charge_saved_card(
        member_id,
        "pm_card_visa",
        12_50,
        "Annual dues",
        "idem-key-1",
        payment_id,
    ).await;

    assert_eq!(result.expect("charge succeeded"), "pi_known_charge");

    let calls = fake.calls();
    assert_eq!(calls.len(), 1, "exactly one gateway call");
    match &calls[0] {
        FakeCall::CreatePaymentIntent(input) => {
            assert_eq!(input.amount_cents, 12_50);
            assert_eq!(input.customer_id, "cus_known");
            assert_eq!(input.payment_method_id, "pm_card_visa");
            assert_eq!(input.idempotency_key, "idem-key-1");
            assert_eq!(input.description, "Annual dues");
            // Metadata carries the member_id + payment_id so the
            // PI.succeeded webhook can resolve the local row.
            assert_eq!(input.metadata.get("member_id").unwrap(), &member_id.to_string());
            assert_eq!(input.metadata.get("payment_id").unwrap(), &payment_id.to_string());
        }
        other => panic!("expected CreatePaymentIntent, got {:?}", other),
    }
}

#[tokio::test]
async fn charge_saved_card_requires_action_returns_external_error() {
    let pool = fresh_pool().await;
    let member_id = insert_member_with_customer(&pool, "cus_3ds").await;
    let (client, fake) = build_client_with_fake(pool);

    fake.next_payment_intent(PaymentIntentResult::RequiresAction {
        id: "pi_3ds_needed".to_string(),
    });

    let err = client.charge_saved_card(
        member_id,
        "pm_card_authentication",
        50_00,
        "Annual dues",
        "idem-key-2",
        Uuid::new_v4(),
    ).await.expect_err("must surface RequiresAction as error");

    match err {
        AppError::External(msg) => assert!(msg.to_lowercase().contains("authentication")),
        other => panic!("expected External, got {:?}", other),
    }
}

#[tokio::test]
async fn charge_saved_card_member_without_customer_bails_before_stripe() {
    let pool = fresh_pool().await;
    // Member exists, but stripe_customer_id is NULL — this is the
    // pre-saved-card state. Charging should refuse before any gateway
    // call goes out.
    let repo = SqliteMemberRepository::new(pool.clone());
    let member = repo.create(CreateMemberRequest {
        email: "no-customer@example.com".to_string(),
        username: "no_customer".to_string(),
        full_name: "No Customer".to_string(),
        password: "p4ssword_long_enough".to_string(),
        membership_type: MembershipType::Regular,
    }).await.unwrap();

    let (client, fake) = build_client_with_fake(pool);
    let err = client.charge_saved_card(
        member.id,
        "pm_card_visa",
        100,
        "x",
        "ikey",
        Uuid::new_v4(),
    ).await.expect_err("must refuse");

    matches!(err, AppError::BadRequest(_));
    assert_eq!(fake.calls().len(), 0, "no Stripe traffic when customer missing");
}

// ---------------------------------------------------------------------
// refund_payment
// ---------------------------------------------------------------------

#[tokio::test]
async fn refund_payment_with_pi_id_passes_through() {
    let pool = fresh_pool().await;
    let (client, fake) = build_client_with_fake(pool);

    // Default fake response is fine — auto-generated re_test_1
    let id = client.refund_payment("pi_real_payment", "ikey-pi").await
        .expect("refund succeeded");

    assert!(id.starts_with("re_test_"), "refund id should be a generated test id, got {}", id);

    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    match &calls[0] {
        FakeCall::CreateRefund(input) => {
            assert_eq!(input.payment_intent_id, "pi_real_payment");
            assert_eq!(input.idempotency_key, "ikey-pi");
        }
        other => panic!("expected CreateRefund, got {:?}", other),
    }
}

#[tokio::test]
async fn refund_payment_with_cs_id_resolves_to_pi_first() {
    let pool = fresh_pool().await;
    let (client, fake) = build_client_with_fake(pool);

    // Queue the cs→pi resolution first, then the refund response.
    fake.next_retrieve_checkout_session(RetrievedCheckoutSession {
        payment_intent_id: Some("pi_from_session".to_string()),
    });
    fake.next_refund(RefundOutput { id: "re_known".to_string() });

    let id = client.refund_payment("cs_test_session", "ikey-cs").await
        .expect("refund succeeded");
    assert_eq!(id, "re_known");

    let calls = fake.calls();
    assert_eq!(calls.len(), 2, "should retrieve session, then create refund");
    assert!(matches!(&calls[0],
        FakeCall::RetrieveCheckoutSession { session_id } if session_id == "cs_test_session"));
    match &calls[1] {
        FakeCall::CreateRefund(input) => {
            assert_eq!(input.payment_intent_id, "pi_from_session",
                "should refund the resolved pi_, not the cs_");
        }
        other => panic!("expected CreateRefund, got {:?}", other),
    }
}

#[tokio::test]
async fn refund_payment_with_invoice_id_resolves_to_pi_first() {
    let pool = fresh_pool().await;
    let (client, fake) = build_client_with_fake(pool);

    fake.next_retrieve_invoice(RetrievedInvoice {
        payment_intent_id: Some("pi_from_invoice".to_string()),
    });

    let id = client.refund_payment("in_subscription_1", "ikey-in").await
        .expect("refund succeeded");
    assert!(id.starts_with("re_test_"));

    let calls = fake.calls();
    assert_eq!(calls.len(), 2);
    assert!(matches!(&calls[0],
        FakeCall::RetrieveInvoice { invoice_id } if invoice_id == "in_subscription_1"));
    match &calls[1] {
        FakeCall::CreateRefund(input) => {
            assert_eq!(input.payment_intent_id, "pi_from_invoice");
        }
        _ => panic!("expected CreateRefund"),
    }
}

#[tokio::test]
async fn refund_payment_rejects_unknown_id_format() {
    let pool = fresh_pool().await;
    let (client, fake) = build_client_with_fake(pool);

    let err = client.refund_payment("xx_unknown_format", "ikey").await
        .expect_err("must reject");
    matches!(err, AppError::BadRequest(_));
    assert_eq!(fake.calls().len(), 0,
        "no Stripe calls should be made for an unrecognized prefix");
}

#[tokio::test]
async fn refund_payment_when_session_has_no_intent_returns_error() {
    let pool = fresh_pool().await;
    let (client, fake) = build_client_with_fake(pool);

    // Stripe sometimes returns a session that hasn't progressed to a
    // PaymentIntent (cancelled, expired, never paid). The refund
    // handler should refuse rather than passing None to the API.
    fake.next_retrieve_checkout_session(RetrievedCheckoutSession {
        payment_intent_id: None,
    });

    let err = client.refund_payment("cs_no_intent", "ikey").await
        .expect_err("must reject session with no PI");
    match err {
        AppError::BadRequest(msg) => {
            assert!(msg.to_lowercase().contains("no paymentintent"),
                "msg: {}", msg);
        }
        other => panic!("expected BadRequest, got {:?}", other),
    }

    // Should have called retrieve_checkout_session exactly once and NOT
    // proceeded to create_refund.
    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    assert!(matches!(&calls[0], FakeCall::RetrieveCheckoutSession { .. }));
}

#[tokio::test]
async fn refund_payment_propagates_stripe_failure() {
    let pool = fresh_pool().await;
    let (client, fake) = build_client_with_fake(pool);

    fake.next_refund_err(AppError::External("Stripe is on fire".to_string()));

    let err = client.refund_payment("pi_anything", "ikey").await
        .expect_err("must surface error");
    match err {
        AppError::External(msg) => assert!(msg.contains("on fire") || msg.contains("Stripe")),
        other => panic!("expected External, got {:?}", other),
    }
}
