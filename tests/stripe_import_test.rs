//! Offline tests for the Stripe payment-history + saved-card backfill,
//! driven entirely through `FakeStripeGateway` (feature `test-utils`) —
//! no real Stripe keys.
//!
//! Run with:  cargo test --features test-utils --test stripe_import_test
//!
//! Covers tasks.md §6:
//!   6.1 backfill idempotency (import once; a re-run adds nothing)
//!   6.2 card de-dup by fingerprint (+ default from Stripe's default pm)
//!   6.3 annual dues statement total matches the imported payments
//!   6.4 receipt email attempted when configured, skipped when not

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use coterie::auth::SecretCrypto;
use coterie::{
    domain::{Payer, Payment, PaymentKind, PaymentMethod, PaymentStatus, StripeRef},
    email::{EmailMessage, EmailSender},
    error::Result as CoterieResult,
    payments::{
        fake_gateway::FakeStripeGateway,
        gateway::{ChargeSummary, PaymentMethodSummary, RetrievedCustomer},
    },
    repository::{
        MemberRepository, PaymentRepository, SavedCardRepository, SqliteMemberRepository,
        SqlitePaymentRepository, SqliteSavedCardRepository,
    },
    service::{
        audit_service::AuditService,
        billing_service::notifications::dispatch_payment_receipt,
        settings_service::{SettingsService, UpdateEmailConfig},
        stripe_import_service::StripeImportService,
    },
    web::portal::payments::receipts::annual_dues_cents,
};
use sqlx::SqlitePool;
use uuid::Uuid;

mod common;
use common::{fresh_pool, make_member};

// --- fixtures --------------------------------------------------------

fn charge(
    id: &str,
    cents: i64,
    created: i64,
    invoice: Option<&str>,
    pi: Option<&str>,
    status: &str,
) -> ChargeSummary {
    ChargeSummary {
        id: id.to_string(),
        amount_cents: cents,
        amount_refunded_cents: 0,
        currency: "usd".to_string(),
        created,
        description: Some("Membership dues".to_string()),
        invoice_id: invoice.map(str::to_string),
        payment_intent_id: pi.map(str::to_string),
        status: status.to_string(),
        metadata: HashMap::new(),
    }
}

/// Mark a charge as partially/fully refunded (net = amount - refunded).
fn with_refund(mut c: ChargeSummary, refunded_cents: i64) -> ChargeSummary {
    c.amount_refunded_cents = refunded_cents;
    c
}

fn card(pm: &str, fingerprint: Option<&str>) -> PaymentMethodSummary {
    PaymentMethodSummary {
        id: pm.to_string(),
        brand: "visa".to_string(),
        last4: "4242".to_string(),
        exp_month: 12,
        exp_year: 2030,
        fingerprint: fingerprint.map(str::to_string),
    }
}

/// Unix seconds for noon on June 15 of `year` — squarely inside the
/// calendar year regardless of timezone, so charges bucket into the
/// year the test expects.
fn mid_year(year: i32) -> i64 {
    chrono::NaiveDate::from_ymd_opt(year, 6, 15)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp()
}

fn build_service(pool: &SqlitePool, gateway: Arc<FakeStripeGateway>) -> StripeImportService {
    let payment_repo: Arc<dyn PaymentRepository> =
        Arc::new(SqlitePaymentRepository::new(pool.clone()));
    let saved_card_repo: Arc<dyn SavedCardRepository> =
        Arc::new(SqliteSavedCardRepository::new(pool.clone()));
    let member_repo: Arc<dyn MemberRepository> =
        Arc::new(SqliteMemberRepository::new(pool.clone()));
    let audit_service = Arc::new(AuditService::new(pool.clone()));
    StripeImportService::new(
        gateway,
        payment_repo,
        saved_card_repo,
        member_repo,
        audit_service,
    )
}

async fn member_with_customer(pool: &SqlitePool, customer_id: &str) -> Uuid {
    let member_id = make_member(pool).await;
    let repo = SqliteMemberRepository::new(pool.clone());
    repo.set_stripe_customer_id(member_id, customer_id)
        .await
        .expect("set stripe_customer_id");
    member_id
}

// --- 6.1 idempotency -------------------------------------------------

#[tokio::test]
async fn backfill_imports_charges_once() {
    let pool = fresh_pool().await;
    let member_id = member_with_customer(&pool, "cus_hist").await;
    let actor = make_member(&pool).await;
    let fake = Arc::new(FakeStripeGateway::new());
    let svc = build_service(&pool, fake.clone());

    // Run 1: two settled charges + one failed (must be skipped).
    fake.next_charges(vec![
        charge(
            "ch_1",
            5000,
            mid_year(2023),
            Some("in_1"),
            Some("pi_1"),
            "succeeded",
        ),
        charge(
            "ch_2",
            5000,
            mid_year(2024),
            Some("in_2"),
            Some("pi_2"),
            "succeeded",
        ),
        // Failed charge WITH keyable ids: the status filter is the only
        // thing that can exclude it, so this actually exercises the filter
        // (not the "no id to key on" skip).
        charge(
            "ch_x",
            9999,
            mid_year(2024),
            Some("in_x"),
            Some("pi_x"),
            "failed",
        ),
    ]);
    let s1 = svc.backfill_all(actor).await.expect("run 1");
    assert_eq!(s1.payments_imported, 2, "two settled charges imported");
    assert_eq!(s1.payments_skipped, 0);

    // Run 2: the same settled charges — nothing new.
    fake.next_charges(vec![
        charge(
            "ch_1",
            5000,
            mid_year(2023),
            Some("in_1"),
            Some("pi_1"),
            "succeeded",
        ),
        charge(
            "ch_2",
            5000,
            mid_year(2024),
            Some("in_2"),
            Some("pi_2"),
            "succeeded",
        ),
    ]);
    let s2 = svc.backfill_all(actor).await.expect("run 2");
    assert_eq!(s2.payments_imported, 0, "re-run imports nothing");
    assert_eq!(s2.payments_skipped, 2, "both recognized as already present");

    // Exactly two payment rows exist.
    let payment_repo: Arc<dyn PaymentRepository> =
        Arc::new(SqlitePaymentRepository::new(pool.clone()));
    let rows = payment_repo.find_by_member(member_id).await.unwrap();
    assert_eq!(rows.len(), 2, "no duplicate rows after re-run");
    assert!(rows
        .iter()
        .all(|p| p.payment_method == PaymentMethod::Stripe));
    assert!(rows.iter().all(|p| p.status == PaymentStatus::Completed));

    // Self-audit: one import_payment per created row + one batch row per
    // run (two runs -> two batch rows).
    let audit = AuditService::new(pool.clone());
    let entries = audit.recent(100).await.unwrap();
    let per_row = entries
        .iter()
        .filter(|e| e.action == "import_payment")
        .count();
    let batches = entries
        .iter()
        .filter(|e| e.action == "import_payments_batch")
        .count();
    assert_eq!(
        per_row, 2,
        "one import_payment audit row per created payment"
    );
    assert_eq!(batches, 2, "one import_payments_batch aggregate per run");

    // The backfill records settled history only — it must never extend
    // dues. The member's dues_paid_until stays exactly as it was.
    let member_repo = SqliteMemberRepository::new(pool.clone());
    let m = member_repo.find_by_id(member_id).await.unwrap().unwrap();
    assert!(
        m.dues_paid_until.is_none(),
        "backfill must not extend member dues"
    );
}

// --- 6.2 card de-dup + default-from-Stripe ---------------------------

#[tokio::test]
async fn backfill_dedups_cards_by_fingerprint_and_sets_stripe_default() {
    let pool = fresh_pool().await;
    let member_id = member_with_customer(&pool, "cus_cards").await;
    let actor = make_member(&pool).await;
    let fake = Arc::new(FakeStripeGateway::new());
    let svc = build_service(&pool, fake.clone());

    // Run 1: two distinct cards. Stripe's default is the SECOND one, so
    // Coterie's default must end up as pm_2 (not the first imported).
    fake.next_charges(vec![]);
    fake.next_payment_methods(vec![card("pm_1", Some("fp_a")), card("pm_2", Some("fp_b"))]);
    fake.next_retrieve_customer(RetrievedCustomer {
        id: "cus_cards".to_string(),
        email: None,
        default_payment_method_id: Some("pm_2".to_string()),
    });
    let s1 = svc.backfill_all(actor).await.expect("card run 1");
    assert_eq!(s1.cards_imported, 2);
    assert_eq!(s1.cards_skipped, 0);

    let card_repo: Arc<dyn SavedCardRepository> =
        Arc::new(SqliteSavedCardRepository::new(pool.clone()));
    let default = card_repo
        .find_default_for_member(member_id)
        .await
        .unwrap()
        .expect("a default card");
    assert_eq!(
        default.stripe_payment_method_id, "pm_2",
        "default matches Stripe's default, not the first imported"
    );

    // Run 2: same fingerprints — even fp_a re-presented under a NEW pm
    // id (pm_3) must be recognized as a duplicate and skipped.
    fake.next_charges(vec![]);
    fake.next_payment_methods(vec![card("pm_3", Some("fp_a")), card("pm_2", Some("fp_b"))]);
    let s2 = svc.backfill_all(actor).await.expect("card run 2");
    assert_eq!(
        s2.cards_imported, 0,
        "duplicate fingerprints not re-inserted"
    );
    assert_eq!(s2.cards_skipped, 2);

    let all = card_repo.find_by_member(member_id).await.unwrap();
    assert_eq!(all.len(), 2, "still exactly two cards");
}

// --- 6.3 annual statement total --------------------------------------

#[tokio::test]
async fn annual_statement_total_matches_imported_payments() {
    let pool = fresh_pool().await;
    let member_id = member_with_customer(&pool, "cus_stmt").await;
    let actor = make_member(&pool).await;
    let fake = Arc::new(FakeStripeGateway::new());
    let svc = build_service(&pool, fake.clone());

    // Two 2023 dues charges + one 2024 dues charge.
    fake.next_charges(vec![
        charge(
            "ch_a",
            4000,
            mid_year(2023),
            Some("in_a"),
            None,
            "succeeded",
        ),
        charge(
            "ch_b",
            6000,
            mid_year(2023),
            Some("in_b"),
            None,
            "succeeded",
        ),
        charge(
            "ch_c",
            5500,
            mid_year(2024),
            Some("in_c"),
            None,
            "succeeded",
        ),
    ]);
    svc.backfill_all(actor).await.expect("import");

    let payment_repo: Arc<dyn PaymentRepository> =
        Arc::new(SqlitePaymentRepository::new(pool.clone()));
    let payments = payment_repo.find_by_member(member_id).await.unwrap();

    // The statement total for 2023 is exactly the sum of the 2023 dues.
    // Fixtures are noon-UTC mid-year, so the year bucket is tz-insensitive.
    let tz = chrono_tz::Tz::UTC;
    assert_eq!(annual_dues_cents(&payments, 2023, tz), 10_000);
    assert_eq!(annual_dues_cents(&payments, 2024, tz), 5_500);
    assert_eq!(annual_dues_cents(&payments, 2022, tz), 0);
}

// --- 6.4 receipt email gating ----------------------------------------

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

fn sample_payment() -> Payment {
    let now = chrono::Utc::now();
    Payment {
        id: Uuid::new_v4(),
        payer: Payer::Member(Uuid::new_v4()),
        amount_cents: 5000,
        currency: "usd".to_string(),
        status: PaymentStatus::Completed,
        payment_method: PaymentMethod::Stripe,
        kind: PaymentKind::Membership,
        external_id: Some(StripeRef::Invoice("in_live".to_string())),
        description: "Annual dues".to_string(),
        paid_at: Some(now),
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
async fn receipt_email_gated_on_configured_provider() {
    let pool = fresh_pool().await;
    let crypto = Arc::new(SecretCrypto::new("test-secret-please-ignore"));
    let settings = SettingsService::new(pool.clone(), crypto);

    let sent = Arc::new(Mutex::new(Vec::new()));
    let sender: Arc<dyn EmailSender> = Arc::new(RecordingSender { sent: sent.clone() });
    let payment = sample_payment();

    // Unconfigured (default mode = "log"): NOTHING is sent.
    dispatch_payment_receipt(
        &sender,
        &settings,
        "http://coterie.test",
        "ann@member.test",
        "Ann Member",
        &payment,
    )
    .await;
    assert_eq!(
        sent.lock().unwrap().len(),
        0,
        "no email attempted when provider unconfigured"
    );

    // Configure a real SMTP provider.
    settings
        .update_email_config(
            UpdateEmailConfig {
                mode: "smtp".to_string(),
                from_address: "billing@org.test".to_string(),
                from_name: "Org".to_string(),
                smtp_host: "smtp.org.test".to_string(),
                smtp_port: 587,
                smtp_username: "user".to_string(),
                smtp_password: Some("secret".to_string()),
            },
            // nil updater → stored as NULL, avoiding the members FK on
            // app_settings.updated_by (no admin needed for this unit).
            Uuid::nil(),
        )
        .await
        .expect("configure smtp");

    dispatch_payment_receipt(
        &sender,
        &settings,
        "http://coterie.test",
        "ann@member.test",
        "Ann Member",
        &payment,
    )
    .await;

    let messages = sent.lock().unwrap();
    assert_eq!(messages.len(), 1, "receipt sent once email is configured");
    assert_eq!(messages[0].to, "ann@member.test");
    assert!(
        messages[0].subject.to_lowercase().contains("receipt"),
        "subject names a receipt: {}",
        messages[0].subject
    );
    assert!(
        messages[0].text_body.contains("$50.00"),
        "receipt body shows the amount"
    );
}

// --- refund netting --------------------------------------------------

#[tokio::test]
async fn backfill_books_net_of_refund_and_skips_fully_refunded() {
    let pool = fresh_pool().await;
    let member_id = member_with_customer(&pool, "cus_refund").await;
    let actor = make_member(&pool).await;
    let fake = Arc::new(FakeStripeGateway::new());
    let svc = build_service(&pool, fake.clone());

    fake.next_charges(vec![
        // Partially refunded: paid 5000, refunded 2000 -> books net 3000.
        with_refund(
            charge("ch_p", 5000, mid_year(2023), Some("in_p"), None, "succeeded"),
            2000,
        ),
        // Fully refunded: nets to 0 -> not a payment, skipped.
        with_refund(
            charge("ch_f", 4000, mid_year(2023), Some("in_f"), None, "succeeded"),
            4000,
        ),
    ]);
    let s = svc.backfill_all(actor).await.expect("import");
    assert_eq!(s.payments_imported, 1, "only the partially-refunded charge books");
    assert_eq!(s.payments_skipped, 1, "fully-refunded charge skipped");

    let payment_repo: Arc<dyn PaymentRepository> =
        Arc::new(SqlitePaymentRepository::new(pool.clone()));
    let payments = payment_repo.find_by_member(member_id).await.unwrap();
    assert_eq!(payments.len(), 1);
    assert_eq!(payments[0].amount_cents, 3000, "booked NET of the refund");
    assert_eq!(
        annual_dues_cents(&payments, 2023, chrono_tz::Tz::UTC),
        3000,
        "statement total reflects net paid, not gross"
    );
}

// --- cross-path idempotency (dedup vs live-recorded payments) ---------

#[tokio::test]
async fn backfill_dedups_against_live_recorded_payment() {
    let pool = fresh_pool().await;
    let member_id = member_with_customer(&pool, "cus_live").await;
    let actor = make_member(&pool).await;
    let payment_repo: Arc<dyn PaymentRepository> =
        Arc::new(SqlitePaymentRepository::new(pool.clone()));

    // The live subscription-invoice webhook already recorded this payment,
    // keyed on invoice id `in_live`.
    let now = chrono::Utc::now();
    payment_repo
        .create(Payment {
            id: Uuid::new_v4(),
            payer: Payer::Member(member_id),
            amount_cents: 5000,
            currency: "usd".to_string(),
            status: PaymentStatus::Completed,
            payment_method: PaymentMethod::Stripe,
            kind: PaymentKind::Membership,
            external_id: Some(StripeRef::Invoice("in_live".to_string())),
            description: "Live subscription payment".to_string(),
            paid_at: Some(now),
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();

    let fake = Arc::new(FakeStripeGateway::new());
    let svc = build_service(&pool, fake.clone());
    // The same invoice surfaces in the historical charge list.
    fake.next_charges(vec![charge(
        "ch_live",
        5000,
        mid_year(2024),
        Some("in_live"),
        Some("pi_live"),
        "succeeded",
    )]);
    let s = svc.backfill_all(actor).await.expect("import");
    assert_eq!(s.payments_imported, 0, "already recorded by the live path");
    assert_eq!(s.payments_skipped, 1);

    let payments = payment_repo.find_by_member(member_id).await.unwrap();
    assert_eq!(payments.len(), 1, "no duplicate of the live-recorded payment");
}

// --- intra-run card fingerprint de-dup -------------------------------

#[tokio::test]
async fn backfill_dedups_cards_within_a_single_run() {
    let pool = fresh_pool().await;
    let member_id = member_with_customer(&pool, "cus_intra").await;
    let actor = make_member(&pool).await;
    let fake = Arc::new(FakeStripeGateway::new());
    let svc = build_service(&pool, fake.clone());

    fake.next_charges(vec![]);
    // Two distinct pm ids sharing ONE fingerprint (same physical card
    // re-attached to the customer) — only one card row should land.
    fake.next_payment_methods(vec![
        card("pm_a", Some("fp_same")),
        card("pm_b", Some("fp_same")),
    ]);
    let s = svc.backfill_all(actor).await.expect("import");
    assert_eq!(s.cards_imported, 1, "shared fingerprint imported once");
    assert_eq!(s.cards_skipped, 1);

    let card_repo: Arc<dyn SavedCardRepository> =
        Arc::new(SqliteSavedCardRepository::new(pool.clone()));
    let all = card_repo.find_by_member(member_id).await.unwrap();
    assert_eq!(all.len(), 1, "one physical card, one row");
}
