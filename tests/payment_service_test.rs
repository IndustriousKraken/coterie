//! Integration tests for `PaymentService::record_manual` — its five
//! validation guards and the audit-action mapping produced via
//! `audit_action(method, kind)`. Hits a real in-memory SQLite +
//! migrations + `AuditService`; constructs `BillingService` against the
//! same pool so the post-work hook is exercisable but stays a no-op
//! when `membership_type_slug` is `None`.
//!
//! Run with: cargo test --test payment_service_test

use std::sync::Arc;

use async_trait::async_trait;
use coterie::{
    auth::SecretCrypto,
    domain::{CreateMemberRequest, PaymentKind, PaymentMethod, MAX_PAYMENT_CENTS},
    email::{EmailMessage, EmailSender},
    error::{AppError, Result as CoterieResult},
    integrations::IntegrationManager,
    repository::{
        DonationCampaignRepository, MemberRepository, PaymentRepository,
        SqliteDonationCampaignRepository, SqliteEventRepository, SqliteMemberRepository,
        SqlitePaymentRepository, SqliteSavedCardRepository, SqliteScheduledPaymentRepository,
    },
    service::{
        audit_service::AuditService,
        billing_service::BillingService,
        membership_type_service::MembershipTypeService,
        payment_service::{PaymentService, RecordManualPaymentInput},
        settings_service::SettingsService,
    },
};
use sqlx::SqlitePool;

mod common;
use common::fresh_pool;
use uuid::Uuid;

// ---------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------

struct NoopEmailSender;

#[async_trait]
impl EmailSender for NoopEmailSender {
    async fn send(&self, _message: &EmailMessage) -> CoterieResult<()> {
        Ok(())
    }
}

struct H {
    pool: SqlitePool,
    payment_service: PaymentService,
    billing: BillingService,
}

async fn build_harness() -> H {
    let pool = fresh_pool().await;

    let member_repo: Arc<dyn MemberRepository> =
        Arc::new(SqliteMemberRepository::new(pool.clone()));
    let payment_repo: Arc<dyn PaymentRepository> =
        Arc::new(SqlitePaymentRepository::new(pool.clone()));
    let campaign_repo: Arc<dyn DonationCampaignRepository> =
        Arc::new(SqliteDonationCampaignRepository::new(pool.clone()));
    let audit_service = Arc::new(AuditService::new(pool.clone()));

    let payment_service = PaymentService::new(
        payment_repo.clone(),
        member_repo.clone(),
        campaign_repo,
        audit_service,
    );

    // BillingService isn't dereferenced by any of these tests (validation
    // failures short-circuit; audit-success cases pass `slug = None`), but
    // `record_manual` takes one by reference, so we construct one wired
    // to the same pool.
    let event_repo = Arc::new(SqliteEventRepository::new(pool.clone()));
    let saved_card_repo = Arc::new(SqliteSavedCardRepository::new(pool.clone()));
    let scheduled_repo = Arc::new(SqliteScheduledPaymentRepository::new(pool.clone()));
    let mt_repo = Arc::new(coterie::repository::SqliteMembershipTypeRepository::new(
        pool.clone(),
    ));
    let mt_service = Arc::new(MembershipTypeService::new(mt_repo));
    let crypto = Arc::new(SecretCrypto::new("test-secret-please-ignore"));
    let settings = Arc::new(SettingsService::new(pool.clone(), crypto));
    let integrations = Arc::new(IntegrationManager::new());
    let email: Arc<dyn EmailSender> = Arc::new(NoopEmailSender);

    let billing = BillingService::new(
        scheduled_repo,
        payment_repo,
        saved_card_repo,
        member_repo,
        event_repo,
        mt_service,
        settings,
        email,
        integrations,
        None,
        "http://localhost:3000".to_string(),
        pool.clone(),
    );

    H {
        pool,
        payment_service,
        billing,
    }
}

async fn seed_member(pool: &SqlitePool) -> Uuid {
    let repo = SqliteMemberRepository::new(pool.clone());
    let member = repo
        .create(CreateMemberRequest {
            email: format!("u-{}@example.com", Uuid::new_v4()),
            username: format!("user_{}", Uuid::new_v4().simple()),
            full_name: "Test Member".to_string(),
            password: "p4ssword_long_enough".to_string(),
            membership_type_id: None,
            ..Default::default()
        })
        .await
        .expect("create member");
    member.id
}

async fn payments_count(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM payments")
        .fetch_one(pool)
        .await
        .expect("count payments")
}

async fn audit_actions_for(pool: &SqlitePool, entity_id: Uuid) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT action FROM audit_logs WHERE entity_type = 'member' AND entity_id = ?",
    )
    .bind(entity_id.to_string())
    .fetch_all(pool)
    .await
    .expect("query audit_logs")
}

// ---------------------------------------------------------------------
// 1. Validation-error tests
// ---------------------------------------------------------------------

#[tokio::test]
async fn record_manual_rejects_negative_amount() {
    let h = build_harness().await;
    let member_id = seed_member(&h.pool).await;
    let actor_id = seed_member(&h.pool).await;

    let err = h
        .payment_service
        .record_manual(
            RecordManualPaymentInput {
                member_id,
                amount_cents: -100,
                kind: PaymentKind::Membership,
                description: "neg".to_string(),
                payment_method: PaymentMethod::Manual,
                membership_type_slug: None,
                actor_id,
            },
            &h.billing,
        )
        .await
        .expect_err("negative amount must reject");

    match err {
        AppError::BadRequest(msg) => {
            assert!(
                msg.contains("amount_cents must not be negative"),
                "unexpected message: {}",
                msg,
            );
        }
        other => panic!("expected BadRequest, got {:?}", other),
    }
    assert_eq!(
        payments_count(&h.pool).await,
        0,
        "no payments row should have been persisted"
    );
}

#[tokio::test]
async fn record_manual_rejects_over_cap_amount() {
    let h = build_harness().await;
    let member_id = seed_member(&h.pool).await;
    let actor_id = seed_member(&h.pool).await;

    let err = h
        .payment_service
        .record_manual(
            RecordManualPaymentInput {
                member_id,
                amount_cents: MAX_PAYMENT_CENTS + 1,
                kind: PaymentKind::Membership,
                description: "over".to_string(),
                payment_method: PaymentMethod::Manual,
                membership_type_slug: None,
                actor_id,
            },
            &h.billing,
        )
        .await
        .expect_err("over-cap amount must reject");

    match err {
        AppError::BadRequest(msg) => {
            let dollars = (MAX_PAYMENT_CENTS / 100).to_string();
            assert!(
                msg.contains(&dollars),
                "message should name the cap in whole dollars ({}), got: {}",
                dollars,
                msg,
            );
        }
        other => panic!("expected BadRequest, got {:?}", other),
    }
    assert_eq!(payments_count(&h.pool).await, 0);
}

#[tokio::test]
async fn record_manual_rejects_stripe_method() {
    let h = build_harness().await;
    let member_id = seed_member(&h.pool).await;
    let actor_id = seed_member(&h.pool).await;

    let err = h
        .payment_service
        .record_manual(
            RecordManualPaymentInput {
                member_id,
                amount_cents: 1_000,
                kind: PaymentKind::Membership,
                description: "stripe-not-here".to_string(),
                payment_method: PaymentMethod::Stripe,
                membership_type_slug: None,
                actor_id,
            },
            &h.billing,
        )
        .await
        .expect_err("Stripe method must reject");

    match err {
        AppError::BadRequest(msg) => {
            assert!(
                msg.contains("Stripe"),
                "message should mention Stripe, got: {}",
                msg,
            );
        }
        other => panic!("expected BadRequest, got {:?}", other),
    }
    assert_eq!(payments_count(&h.pool).await, 0);
}

#[tokio::test]
async fn record_manual_rejects_unknown_member() {
    let h = build_harness().await;
    let actor_id = seed_member(&h.pool).await;
    let unknown_member = Uuid::new_v4();

    let err = h
        .payment_service
        .record_manual(
            RecordManualPaymentInput {
                member_id: unknown_member,
                amount_cents: 500,
                kind: PaymentKind::Membership,
                description: "no-such-member".to_string(),
                payment_method: PaymentMethod::Manual,
                membership_type_slug: None,
                actor_id,
            },
            &h.billing,
        )
        .await
        .expect_err("unknown member must reject");

    match err {
        AppError::BadRequest(msg) => {
            assert!(
                msg.contains(&unknown_member.to_string()),
                "message should include the unknown id ({}), got: {}",
                unknown_member,
                msg,
            );
        }
        other => panic!("expected BadRequest, got {:?}", other),
    }
    assert_eq!(payments_count(&h.pool).await, 0);
}

#[tokio::test]
async fn record_manual_rejects_donation_with_stale_campaign_id() {
    let h = build_harness().await;
    let member_id = seed_member(&h.pool).await;
    let actor_id = seed_member(&h.pool).await;
    let stale_campaign = Uuid::new_v4();

    let err = h
        .payment_service
        .record_manual(
            RecordManualPaymentInput {
                member_id,
                amount_cents: 2_500,
                kind: PaymentKind::Donation {
                    campaign_id: Some(stale_campaign),
                },
                description: "ghost-campaign".to_string(),
                payment_method: PaymentMethod::Manual,
                membership_type_slug: None,
                actor_id,
            },
            &h.billing,
        )
        .await
        .expect_err("stale campaign id must reject");

    assert!(
        matches!(err, AppError::BadRequest(_)),
        "expected BadRequest, got {:?}",
        err,
    );
    assert_eq!(
        payments_count(&h.pool).await,
        0,
        "no orphan donation row should have been created"
    );
}

// ---------------------------------------------------------------------
// 2. Audit-action mapping tests
// ---------------------------------------------------------------------

#[tokio::test]
async fn record_manual_waived_dues_audits_as_waive_dues() {
    let h = build_harness().await;
    let member_id = seed_member(&h.pool).await;
    let actor_id = seed_member(&h.pool).await;

    h.payment_service
        .record_manual(
            RecordManualPaymentInput {
                member_id,
                amount_cents: 0,
                kind: PaymentKind::Membership,
                description: "comp'd this quarter".to_string(),
                payment_method: PaymentMethod::Waived,
                membership_type_slug: None,
                actor_id,
            },
            &h.billing,
        )
        .await
        .expect("waived membership should record");

    let actions = audit_actions_for(&h.pool, member_id).await;
    assert_eq!(
        actions.len(),
        1,
        "expected exactly one audit row for this member, got {:?}",
        actions
    );
    assert_eq!(actions[0], "waive_dues");
}

#[tokio::test]
async fn record_manual_cash_membership_audits_as_manual_payment() {
    let h = build_harness().await;
    let member_id = seed_member(&h.pool).await;
    let actor_id = seed_member(&h.pool).await;

    h.payment_service
        .record_manual(
            RecordManualPaymentInput {
                member_id,
                amount_cents: 10_00,
                kind: PaymentKind::Membership,
                description: "cash in hand".to_string(),
                payment_method: PaymentMethod::Manual,
                membership_type_slug: None,
                actor_id,
            },
            &h.billing,
        )
        .await
        .expect("manual membership should record");

    let actions = audit_actions_for(&h.pool, member_id).await;
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0], "manual_payment");
}

#[tokio::test]
async fn record_manual_donation_audits_as_manual_donation() {
    let h = build_harness().await;
    let member_id = seed_member(&h.pool).await;
    let actor_id = seed_member(&h.pool).await;

    h.payment_service
        .record_manual(
            RecordManualPaymentInput {
                member_id,
                amount_cents: 25_00,
                kind: PaymentKind::Donation { campaign_id: None },
                description: "general fund".to_string(),
                payment_method: PaymentMethod::Manual,
                membership_type_slug: None,
                actor_id,
            },
            &h.billing,
        )
        .await
        .expect("manual donation should record");

    let actions = audit_actions_for(&h.pool, member_id).await;
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0], "manual_donation");
}
