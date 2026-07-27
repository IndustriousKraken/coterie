//! Paid-event registration: the seat/charge/release state machine.
//!
//! The invariant under test throughout: never hold money for a seat
//! that does not exist, and never hold a seat nobody paid for. Every
//! test here is an ordering or idempotency check on that.
//!
//! Run with: cargo test --features test-utils --test paid_events_test

use std::net::IpAddr;
use std::sync::Arc;

use chrono::{Duration, Utc};
use coterie::{
    api::state::{MoneyLimiter, RateLimiter},
    auth::SecretCrypto,
    domain::{
        AttendanceStatus, CreateMemberRequest, Event, EventType, EventVisibility, Member,
        MemberStatus, Payer, Payment, PaymentKind, PaymentMethod, PaymentStatus, StripeRef,
    },
    email::LogSender,
    error::AppError,
    integrations::IntegrationManager,
    payments::{
        fake_gateway::FakeStripeGateway, gateway::StripeGateway, StripeClient, StripeHandle,
        WebhookDispatcher,
    },
    repository::{
        EventRepository, MemberRepository, PaymentRepository, SqliteEventRepository,
        SqliteMemberRepository, SqlitePaymentRepository, SqliteSavedCardRepository,
        SqliteScheduledPaymentRepository,
    },
    service::{
        audit_service::AuditService,
        billing_service::BillingService,
        event_registration_service::{EventRegistrationService, RegistrationOutcome},
        membership_type_service::MembershipTypeService,
        payment_admin_service::{PaymentAdminService, RefundError},
        payment_service::{PaymentService, RecordManualPaymentInput},
        settings_service::SettingsService,
    },
};
use serde_json::json;
use sqlx::SqlitePool;
use uuid::Uuid;

mod common;
use common::fresh_pool;

// ---------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------

struct Harness {
    pool: SqlitePool,
    event_repo: Arc<dyn EventRepository>,
    payment_repo: Arc<dyn PaymentRepository>,
    member_repo: Arc<dyn MemberRepository>,
    registration: Arc<EventRegistrationService>,
    dispatcher: WebhookDispatcher,
    payment_admin: PaymentAdminService,
    payment_service: PaymentService,
    billing: BillingService,
    fake: Arc<FakeStripeGateway>,
}

async fn build_harness() -> Harness {
    let pool = fresh_pool().await;
    let fake = Arc::new(FakeStripeGateway::new());
    let gw: Arc<dyn StripeGateway> = fake.clone();

    let payment_repo: Arc<dyn PaymentRepository> =
        Arc::new(SqlitePaymentRepository::new(pool.clone()));
    let member_repo: Arc<dyn MemberRepository> =
        Arc::new(SqliteMemberRepository::new(pool.clone()));
    let event_repo: Arc<dyn EventRepository> = Arc::new(SqliteEventRepository::new(pool.clone()));
    let audit_service = Arc::new(AuditService::new(pool.clone()));
    let integrations = Arc::new(IntegrationManager::new());
    let mt_service = Arc::new(MembershipTypeService::new(Arc::new(
        coterie::repository::SqliteMembershipTypeRepository::new(pool.clone()),
    )));
    let crypto = Arc::new(SecretCrypto::new("test-secret-please-ignore"));
    let settings = Arc::new(SettingsService::new(pool.clone(), crypto));
    let email_sender = Arc::new(LogSender::new(
        "test@example.com".to_string(),
        "Test".to_string(),
    ));
    let donation_campaign_repo: Arc<dyn coterie::repository::DonationCampaignRepository> = Arc::new(
        coterie::repository::SqliteDonationCampaignRepository::new(pool.clone()),
    );

    let client = Arc::new(StripeClient::with_gateway(
        gw.clone(),
        payment_repo.clone(),
        member_repo.clone(),
    ));
    let stripe_handle = Arc::new(StripeHandle::preloaded(Some(client), None));

    let processed_events_repo: Arc<dyn coterie::repository::ProcessedEventsRepository> = Arc::new(
        coterie::repository::SqliteProcessedEventsRepository::new(pool.clone()),
    );
    let dispatcher = WebhookDispatcher::new(
        gw,
        "whsec_test_dummy".to_string(),
        payment_repo.clone(),
        member_repo.clone(),
        event_repo.clone(),
        processed_events_repo,
        mt_service.clone(),
        integrations.clone(),
        audit_service.clone(),
    );

    let billing = BillingService::new(
        Arc::new(SqliteScheduledPaymentRepository::new(pool.clone())),
        payment_repo.clone(),
        Arc::new(SqliteSavedCardRepository::new(pool.clone())),
        member_repo.clone(),
        event_repo.clone(),
        mt_service,
        settings,
        email_sender,
        integrations.clone(),
        stripe_handle.clone(),
        "http://localhost:3000".to_string(),
        pool.clone(),
    );

    let payment_admin = PaymentAdminService::new(
        payment_repo.clone(),
        event_repo.clone(),
        stripe_handle.clone(),
        audit_service.clone(),
        integrations.clone(),
        MoneyLimiter(RateLimiter::new(1000, std::time::Duration::from_secs(60))),
    );

    let payment_service = PaymentService::new(
        payment_repo.clone(),
        member_repo.clone(),
        donation_campaign_repo,
        audit_service,
    );

    let registration = Arc::new(EventRegistrationService::new(
        event_repo.clone(),
        payment_repo.clone(),
        stripe_handle,
        "http://localhost:3000".to_string(),
    ));

    Harness {
        pool,
        event_repo,
        payment_repo,
        member_repo,
        registration,
        dispatcher,
        payment_admin,
        payment_service,
        billing,
        fake,
    }
}

impl Harness {
    /// An Active member — the RSVP rule only admits Active/Honorary.
    async fn active_member(&self, tag: &str) -> Member {
        let mut m = self
            .member_repo
            .create(CreateMemberRequest {
                email: format!("{tag}-{}@example.com", Uuid::new_v4()),
                username: format!("u_{}", Uuid::new_v4().simple()),
                full_name: format!("Member {tag}"),
                password: "p4ssword_long_enough".to_string(),
                membership_type_id: None,
                ..Default::default()
            })
            .await
            .unwrap();
        sqlx::query("UPDATE members SET status = 'Active' WHERE id = ?")
            .bind(m.id.to_string())
            .execute(&self.pool)
            .await
            .unwrap();
        m.status = MemberStatus::Active;
        m
    }

    async fn event(&self, creator: Uuid, price_cents: i64, max_attendees: Option<i32>) -> Event {
        let now = Utc::now();
        self.event_repo
            .create(Event {
                id: Uuid::new_v4(),
                title: "Lockpicking 101".to_string(),
                description: "Bring a padlock".to_string(),
                event_type: EventType::Workshop,
                event_type_id: None,
                visibility: EventVisibility::MembersOnly,
                start_time: now + Duration::days(7),
                end_time: None,
                timezone: "UTC".to_string(),
                location: None,
                max_attendees,
                rsvp_required: true,
                member_price_cents: price_cents,
                image_url: None,
                created_by: creator,
                created_at: now,
                updated_at: now,
                series_id: None,
                occurrence_index: None,
            })
            .await
            .unwrap()
    }

    async fn attendance_status(&self, event_id: Uuid, member_id: Uuid) -> Option<AttendanceStatus> {
        self.event_repo
            .get_member_attendance_status(event_id, member_id)
            .await
            .unwrap()
    }

    async fn payment_count(&self) -> i64 {
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM payments")
            .fetch_one(&self.pool)
            .await
            .unwrap();
        n
    }

    async fn audit_count(&self, action: &str) -> i64 {
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM audit_logs WHERE action = ?")
            .bind(action)
            .fetch_one(&self.pool)
            .await
            .unwrap();
        n
    }
}

fn loopback() -> IpAddr {
    IpAddr::from([127, 0, 0, 1])
}

/// A `checkout.session.completed` payload for an event-fee session.
fn event_fee_session(
    id: &str,
    event_id: Uuid,
    payment_intent: Option<&str>,
) -> stripe::CheckoutSession {
    let body = json!({
        "id": id,
        "object": "checkout.session",
        "livemode": false,
        "mode": "payment",
        "status": "complete",
        "payment_status": "paid",
        "created": Utc::now().timestamp(),
        "expires_at": Utc::now().timestamp() + 3600,
        "currency": "usd",
        "amount_total": 3000,
        "amount_subtotal": 3000,
        "metadata": {
            "payment_type": "event_fee",
            "event_id": event_id.to_string(),
        },
        "payment_intent": payment_intent,
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

fn expired_session(id: &str) -> stripe::CheckoutSession {
    let body = json!({
        "id": id,
        "object": "checkout.session",
        "livemode": false,
        "mode": "payment",
        "status": "expired",
        "payment_status": "unpaid",
        "created": Utc::now().timestamp(),
        "expires_at": Utc::now().timestamp(),
        "currency": "usd",
        "metadata": { "payment_type": "event_fee" },
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

fn charge(id: &str, amount: i64, payment_intent: &str) -> stripe::Charge {
    let body = json!({
        "id": id,
        "object": "charge",
        "amount": amount,
        "amount_captured": amount,
        "amount_refunded": amount,
        "billing_details": { "address": null, "email": null, "name": null, "phone": null },
        "currency": "usd",
        "captured": true,
        "created": Utc::now().timestamp(),
        "disputed": false,
        "livemode": false,
        "paid": true,
        "refunded": true,
        "status": "succeeded",
        "payment_intent": payment_intent,
        "metadata": {},
    });
    serde_json::from_value(body).expect("Charge from JSON")
}

// ---------------------------------------------------------------------
// 7.1 Race: two registrations for the last seat
// ---------------------------------------------------------------------

#[tokio::test]
async fn last_seat_race_produces_exactly_one_winner() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let event = h.event(creator.id, 3000, Some(1)).await;
    let a = h.active_member("a").await;
    let b = h.active_member("b").await;

    // Both in flight at once. The test pool is a single connection, so
    // the two tasks interleave at their await points and serialize on
    // the write — which is exactly the shape a lost race takes: the
    // second caller must observe the seat as already held.
    let (svc1, svc2) = (h.registration.clone(), h.registration.clone());
    let (ev1, ev2) = (event.clone(), event.clone());
    let (ma, mb) = (a.clone(), b.clone());
    let t1 = tokio::spawn(async move { svc1.register(&ma, &ev1).await });
    let t2 = tokio::spawn(async move { svc2.register(&mb, &ev2).await });
    let results = [t1.await.unwrap(), t2.await.unwrap()];

    let winners = results
        .iter()
        .filter(|r| matches!(r, Ok(RegistrationOutcome::Checkout { .. })))
        .count();
    let losers = results
        .iter()
        .filter(|r| matches!(r, Err(AppError::BadRequest(msg)) if msg.contains("full")))
        .count();
    assert_eq!(
        winners, 1,
        "exactly one registration reaches Checkout: {results:?}"
    );
    assert_eq!(losers, 1, "the other gets BadRequest: {results:?}");

    // The loser leaves no trace: no seat, no payment row.
    let seated: Vec<_> = h.event_repo.roster(event.id).await.unwrap();
    assert_eq!(seated.len(), 1, "only the winner has an attendance row");
    assert_eq!(
        h.payment_count().await,
        1,
        "only the winner's payment exists"
    );
    assert_eq!(h.event_repo.count_held_seats(event.id).await.unwrap(), 1);
}

// ---------------------------------------------------------------------
// 7.2 Abandoned checkout frees the seat
// ---------------------------------------------------------------------

#[tokio::test]
async fn expired_checkout_releases_the_seat_for_the_next_member() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let event = h.event(creator.id, 3000, Some(1)).await;
    let a = h.active_member("a").await;
    let b = h.active_member("b").await;

    h.registration.register(&a, &event).await.unwrap();
    assert_eq!(h.event_repo.count_held_seats(event.id).await.unwrap(), 1);

    // The event is full for B while A is at Stripe.
    assert!(h.registration.register(&b, &event).await.is_err());

    // A abandons: Stripe expires the session. No seat-specific code
    // runs — the payment leaving Pending is what frees the seat.
    let pending = h
        .payment_repo
        .find_event_fee_payment(event.id, a.id)
        .await
        .unwrap()
        .unwrap();
    let session_id = pending.external_id.as_ref().unwrap().as_str().to_string();
    h.dispatcher
        .dispatch_expired_session(expired_session(&session_id))
        .await
        .unwrap();

    let after = h
        .payment_repo
        .find_by_id(pending.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.status, PaymentStatus::Failed);
    assert_eq!(
        h.event_repo.count_held_seats(event.id).await.unwrap(),
        0,
        "a PendingPayment row whose payment left Pending holds no seat",
    );

    // The abandoned attempt stays visible on the roster.
    let roster = h.event_repo.roster(event.id).await.unwrap();
    assert_eq!(roster.len(), 1);
    assert_eq!(roster[0].status, AttendanceStatus::PendingPayment);
    assert_eq!(roster[0].payment_status, Some(PaymentStatus::Failed));

    // And B can now register.
    assert!(matches!(
        h.registration.register(&b, &event).await,
        Ok(RegistrationOutcome::Checkout { .. })
    ));
}

// ---------------------------------------------------------------------
// 7.3 Double-charge guard
// ---------------------------------------------------------------------

#[tokio::test]
async fn registering_again_after_paying_charges_nothing() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let event = h.event(creator.id, 3000, None).await;
    let m = h.active_member("m").await;

    h.registration.register(&m, &event).await.unwrap();
    let payment = h
        .payment_repo
        .find_event_fee_payment(event.id, m.id)
        .await
        .unwrap()
        .unwrap();
    let session_id = payment.external_id.as_ref().unwrap().as_str().to_string();
    h.dispatcher
        .dispatch_checkout_session_completed(
            event_fee_session(&session_id, event.id, Some("pi_paid_1")),
            &h.billing,
        )
        .await
        .unwrap();
    assert_eq!(
        h.attendance_status(event.id, m.id).await,
        Some(AttendanceStatus::Registered)
    );

    let sessions_before = h.fake.count_where(|c| {
        matches!(
            c,
            coterie::payments::fake_gateway::FakeCall::CreateCheckoutSession(_)
        )
    });

    let again = h.registration.register(&m, &event).await.unwrap();
    assert_eq!(again, RegistrationOutcome::Registered);
    assert_eq!(h.payment_count().await, 1, "no second payment row");
    assert_eq!(
        h.fake.count_where(|c| matches!(
            c,
            coterie::payments::fake_gateway::FakeCall::CreateCheckoutSession(_)
        )),
        sessions_before,
        "no second Checkout session",
    );
}

#[tokio::test]
async fn double_submit_reuses_the_in_flight_checkout() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let event = h.event(creator.id, 3000, Some(1)).await;
    let m = h.active_member("m").await;

    let first = h.registration.register(&m, &event).await.unwrap();
    let RegistrationOutcome::Checkout { url: first_url } = first else {
        panic!("expected a checkout redirect");
    };

    // Stripe reports the session as still open, so it's reusable.
    h.fake
        .next_retrieve_checkout_session(coterie::payments::gateway::RetrievedCheckoutSession {
            payment_intent_id: None,
            is_open: true,
            url: Some(first_url.clone()),
        });

    let second = h.registration.register(&m, &event).await.unwrap();
    assert_eq!(second, RegistrationOutcome::Checkout { url: first_url });
    assert_eq!(h.payment_count().await, 1, "no second payment row");
    assert_eq!(
        h.event_repo.count_held_seats(event.id).await.unwrap(),
        1,
        "no second seat claimed",
    );
}

// ---------------------------------------------------------------------
// 7.4 Webhook completion: idempotent, and no dues side effects
// ---------------------------------------------------------------------

#[tokio::test]
async fn completion_is_idempotent_and_does_not_extend_dues() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let event = h.event(creator.id, 3000, None).await;
    let m = h.active_member("m").await;

    let dues_before = h
        .member_repo
        .find_by_id(m.id)
        .await
        .unwrap()
        .unwrap()
        .dues_paid_until;

    h.registration.register(&m, &event).await.unwrap();
    let payment = h
        .payment_repo
        .find_event_fee_payment(event.id, m.id)
        .await
        .unwrap()
        .unwrap();
    let session_id = payment.external_id.as_ref().unwrap().as_str().to_string();
    let session = event_fee_session(&session_id, event.id, Some("pi_paid_2"));

    h.dispatcher
        .dispatch_checkout_session_completed(session.clone(), &h.billing)
        .await
        .unwrap();
    // Stripe's at-least-once delivery: the same event again.
    h.dispatcher
        .dispatch_checkout_session_completed(session, &h.billing)
        .await
        .unwrap();

    let after = h
        .payment_repo
        .find_by_id(payment.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.status, PaymentStatus::Completed);
    assert_eq!(
        h.attendance_status(event.id, m.id).await,
        Some(AttendanceStatus::Registered)
    );
    assert_eq!(h.payment_count().await, 1, "no duplicate payment row");
    assert_eq!(
        h.audit_count("event_registration_paid").await,
        1,
        "redelivery must not write a second audit row",
    );

    let dues_after = h
        .member_repo
        .find_by_id(m.id)
        .await
        .unwrap()
        .unwrap()
        .dues_paid_until;
    assert_eq!(dues_before, dues_after, "an event fee is not dues");
}

// ---------------------------------------------------------------------
// 7.5 Refund releases the seat — both routes
// ---------------------------------------------------------------------

#[tokio::test]
async fn admin_refund_cancels_the_seat() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let event = h.event(creator.id, 3000, Some(1)).await;
    let m = h.active_member("m").await;

    // A manual (at-the-door) event fee: refundable with no Stripe call.
    let payment = h
        .payment_service
        .record_manual(
            RecordManualPaymentInput {
                member_id: m.id,
                amount_cents: 3000,
                kind: PaymentKind::EventFee { event_id: event.id },
                description: "Event registration — Lockpicking 101".to_string(),
                payment_method: PaymentMethod::Manual,
                membership_type_slug: None,
                actor_id: creator.id,
            },
            &h.billing,
        )
        .await
        .unwrap();
    h.event_repo
        .register_attendance(event.id, m.id)
        .await
        .unwrap();
    h.event_repo
        .link_payment(event.id, m.id, payment.id)
        .await
        .unwrap();
    assert_eq!(h.event_repo.count_held_seats(event.id).await.unwrap(), 1);

    h.payment_admin
        .refund(creator.id, payment.id, loopback())
        .await
        .unwrap();

    assert_eq!(
        h.payment_repo
            .find_by_id(payment.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        PaymentStatus::Refunded
    );
    assert_eq!(
        h.attendance_status(event.id, m.id).await,
        Some(AttendanceStatus::Cancelled)
    );
    assert_eq!(
        h.event_repo.count_held_seats(event.id).await.unwrap(),
        0,
        "the seat is available to another member",
    );
    assert_eq!(h.audit_count("event_registration_refunded").await, 1);
}

#[tokio::test]
async fn out_of_band_charge_refunded_cancels_the_seat() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let event = h.event(creator.id, 3000, Some(1)).await;
    let m = h.active_member("m").await;

    // A completed Stripe event fee, keyed by PaymentIntent as the
    // completion webhook leaves it.
    let now = Utc::now();
    let payment = h
        .payment_repo
        .create(Payment {
            id: Uuid::new_v4(),
            payer: Payer::Member(m.id),
            amount_cents: 3000,
            currency: "USD".to_string(),
            status: PaymentStatus::Completed,
            payment_method: PaymentMethod::Stripe,
            kind: PaymentKind::EventFee { event_id: event.id },
            external_id: Some(StripeRef::PaymentIntent("pi_oob".to_string())),
            description: "Event registration — Lockpicking 101".to_string(),
            paid_at: Some(now),
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    h.event_repo
        .register_attendance(event.id, m.id)
        .await
        .unwrap();
    h.event_repo
        .link_payment(event.id, m.id, payment.id)
        .await
        .unwrap();

    h.dispatcher
        .dispatch_charge_refunded(charge("ch_oob", 3000, "pi_oob"))
        .await
        .unwrap();

    assert_eq!(
        h.payment_repo
            .find_by_id(payment.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        PaymentStatus::Refunded
    );
    assert_eq!(
        h.attendance_status(event.id, m.id).await,
        Some(AttendanceStatus::Cancelled)
    );
    assert_eq!(h.event_repo.count_held_seats(event.id).await.unwrap(), 0);
}

// ---------------------------------------------------------------------
// 7.6 Deleting a paid event refunds first; a failure aborts
// ---------------------------------------------------------------------

async fn seat_a_paid_attendee(
    h: &Harness,
    event: &Event,
    actor: Uuid,
    tag: &str,
) -> (Member, Payment) {
    let m = h.active_member(tag).await;
    let payment = h
        .payment_service
        .record_manual(
            RecordManualPaymentInput {
                member_id: m.id,
                amount_cents: 3000,
                kind: PaymentKind::EventFee { event_id: event.id },
                description: "Event registration — Lockpicking 101".to_string(),
                payment_method: PaymentMethod::Manual,
                membership_type_slug: None,
                actor_id: actor,
            },
            &h.billing,
        )
        .await
        .unwrap();
    h.event_repo
        .register_attendance(event.id, m.id)
        .await
        .unwrap();
    h.event_repo
        .link_payment(event.id, m.id, payment.id)
        .await
        .unwrap();
    (m, payment)
}

#[tokio::test]
async fn deleting_a_paid_event_refunds_every_attendee_first() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let event = h.event(creator.id, 3000, None).await;

    let mut members = Vec::new();
    for tag in ["a", "b", "c"] {
        members.push(seat_a_paid_attendee(&h, &event, creator.id, tag).await);
    }

    let refunded = h
        .payment_admin
        .refund_all_event_fees(creator.id, event.id, loopback())
        .await
        .unwrap();
    assert_eq!(refunded, 3);

    for (m, p) in &members {
        assert_eq!(
            h.payment_repo
                .find_by_id(p.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            PaymentStatus::Refunded
        );
        assert_eq!(
            h.attendance_status(event.id, m.id).await,
            Some(AttendanceStatus::Cancelled)
        );
    }
    assert_eq!(h.audit_count("refund_event_fees_bulk").await, 1);

    // Only after the refunds does the event go.
    h.event_repo.delete(event.id).await.unwrap();
    assert!(h.event_repo.find_by_id(event.id).await.unwrap().is_none());
}

#[tokio::test]
async fn a_failing_refund_aborts_the_delete_and_leaves_the_roster_intact() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let event = h.event(creator.id, 3000, None).await;
    let m = h.active_member("m").await;

    // A Stripe-method fee whose refund Stripe will reject.
    let now = Utc::now();
    let payment = h
        .payment_repo
        .create(Payment {
            id: Uuid::new_v4(),
            payer: Payer::Member(m.id),
            amount_cents: 3000,
            currency: "USD".to_string(),
            status: PaymentStatus::Completed,
            payment_method: PaymentMethod::Stripe,
            kind: PaymentKind::EventFee { event_id: event.id },
            external_id: Some(StripeRef::PaymentIntent("pi_broken".to_string())),
            description: "Event registration — Lockpicking 101".to_string(),
            paid_at: Some(now),
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    h.event_repo
        .register_attendance(event.id, m.id)
        .await
        .unwrap();
    h.event_repo
        .link_payment(event.id, m.id, payment.id)
        .await
        .unwrap();

    h.fake
        .next_refund_err(AppError::External("card network down".to_string()));

    let err = h
        .payment_admin
        .refund_all_event_fees(creator.id, event.id, loopback())
        .await
        .unwrap_err();
    assert!(matches!(err, RefundError::StripeApiError(_)), "got {err:?}");

    // The caller must NOT delete: everything is exactly as it was.
    assert!(h.event_repo.find_by_id(event.id).await.unwrap().is_some());
    assert_eq!(
        h.payment_repo
            .find_by_id(payment.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        PaymentStatus::Completed,
        "a failed Stripe refund unclaims the row",
    );
    assert_eq!(
        h.attendance_status(event.id, m.id).await,
        Some(AttendanceStatus::Registered),
        "the roster is intact",
    );
}

// ---------------------------------------------------------------------
// 7.7 Free events are unchanged
// ---------------------------------------------------------------------

#[tokio::test]
async fn free_event_registration_touches_no_payment_machinery() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    // max_attendees of 1 — capacity stays ADVISORY for free events.
    let event = h.event(creator.id, 0, Some(1)).await;
    let a = h.active_member("a").await;
    let b = h.active_member("b").await;

    assert_eq!(
        h.registration.register(&a, &event).await.unwrap(),
        RegistrationOutcome::Registered
    );
    assert_eq!(
        h.attendance_status(event.id, a.id).await,
        Some(AttendanceStatus::Registered)
    );
    assert_eq!(h.payment_count().await, 0, "no payment row");
    assert_eq!(
        h.fake.count_where(|c| matches!(
            c,
            coterie::payments::fake_gateway::FakeCall::CreateCheckoutSession(_)
        )),
        0,
        "no Checkout session",
    );

    // Over capacity, and still accepted — today's advisory behavior.
    assert_eq!(
        h.registration.register(&b, &event).await.unwrap(),
        RegistrationOutcome::Registered
    );

    // And no audit row for a free RSVP.
    assert_eq!(h.audit_count("event_registration_paid").await, 0);
}

#[tokio::test]
async fn a_blank_price_stores_zero_and_free_events_are_findable_by_equality() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let free = h.event(creator.id, 0, None).await;
    let paid = h.event(creator.id, 2500, None).await;

    assert!(!free.is_paid_for_members());
    assert!(paid.is_paid_for_members());

    // The obvious queries find free events — the whole reason 0 is
    // stored as 0 rather than NULL.
    let (eq,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM events WHERE member_price_cents = 0")
        .fetch_one(&h.pool)
        .await
        .unwrap();
    assert_eq!(eq, 1);
    let (range,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM events WHERE member_price_cents <= 2000")
            .fetch_one(&h.pool)
            .await
            .unwrap();
    assert_eq!(range, 1, "a range query must include free events");
    let (nulls,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM events WHERE member_price_cents IS NULL")
            .fetch_one(&h.pool)
            .await
            .unwrap();
    assert_eq!(nulls, 0, "no price is ever stored as NULL");
}

#[tokio::test]
async fn raising_the_price_does_not_rebill_existing_attendees() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let event = h.event(creator.id, 3000, None).await;
    let (m, payment) = seat_a_paid_attendee(&h, &event, creator.id, "m").await;

    let mut raised = event.clone();
    raised.member_price_cents = 9000;
    h.event_repo.update(event.id, raised).await.unwrap();

    assert_eq!(
        h.attendance_status(event.id, m.id).await,
        Some(AttendanceStatus::Registered)
    );
    let after = h
        .payment_repo
        .find_by_id(payment.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.amount_cents, 3000, "the settled amount is unchanged");
    assert_eq!(h.payment_count().await, 1, "no top-up charge");
}

// ---------------------------------------------------------------------
// 7.8 Audit action strings
// ---------------------------------------------------------------------

#[tokio::test]
async fn event_fee_audit_actions_do_not_collide_with_dues() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let event = h.event(creator.id, 3000, None).await;
    let cash = h.active_member("cash").await;
    let comped = h.active_member("comped").await;
    let waived_dues = h.active_member("dues").await;

    for (member, amount, method, kind) in [
        (
            cash.id,
            3000,
            PaymentMethod::Manual,
            PaymentKind::EventFee { event_id: event.id },
        ),
        (
            comped.id,
            0,
            PaymentMethod::Waived,
            PaymentKind::EventFee { event_id: event.id },
        ),
        (
            waived_dues.id,
            0,
            PaymentMethod::Waived,
            PaymentKind::Membership,
        ),
    ] {
        h.payment_service
            .record_manual(
                RecordManualPaymentInput {
                    member_id: member,
                    amount_cents: amount,
                    kind,
                    description: "test".to_string(),
                    payment_method: method,
                    membership_type_slug: None,
                    actor_id: creator.id,
                },
                &h.billing,
            )
            .await
            .unwrap();
    }

    assert_eq!(h.audit_count("manual_event_fee").await, 1);
    assert_eq!(
        h.audit_count("waive_event_fee").await,
        1,
        "a comped seat must not be absorbed by the dues-waiver arm",
    );
    assert_eq!(
        h.audit_count("waive_dues").await,
        1,
        "the pre-existing membership mapping still holds",
    );
}

// ---------------------------------------------------------------------
// Ordering: a failed session release the claimed seat
// ---------------------------------------------------------------------

#[tokio::test]
async fn session_creation_failure_releases_the_claimed_seat() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let event = h.event(creator.id, 3000, Some(1)).await;
    let a = h.active_member("a").await;
    let b = h.active_member("b").await;

    h.fake
        .next_checkout_session_err(AppError::External("stripe is down".to_string()));

    assert!(h.registration.register(&a, &event).await.is_err());
    assert!(
        h.attendance_status(event.id, a.id).await.is_none(),
        "a seat that can never be paid for must not stay held",
    );
    assert_eq!(h.event_repo.count_held_seats(event.id).await.unwrap(), 0);

    // The last seat is still available to somebody else.
    assert!(matches!(
        h.registration.register(&b, &event).await,
        Ok(RegistrationOutcome::Checkout { .. })
    ));
}

#[tokio::test]
async fn a_non_active_member_cannot_claim_a_seat() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let event = h.event(creator.id, 3000, Some(1)).await;
    let mut expired = h.active_member("expired").await;
    expired.status = MemberStatus::Expired;

    assert!(h.registration.register(&expired, &event).await.is_err());
    assert_eq!(h.event_repo.count_held_seats(event.id).await.unwrap(), 0);
    assert_eq!(h.payment_count().await, 0);
}

// ---------------------------------------------------------------------
// Checkout session shape: metadata + bounded expiry
// ---------------------------------------------------------------------

#[tokio::test]
async fn event_fee_session_is_stamped_and_expires_within_an_hour() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let event = h.event(creator.id, 3000, None).await;
    let m = h.active_member("m").await;

    h.registration.register(&m, &event).await.unwrap();

    let calls = h.fake.calls();
    let input = calls
        .iter()
        .find_map(|c| match c {
            coterie::payments::fake_gateway::FakeCall::CreateCheckoutSession(i) => Some(i.clone()),
            _ => None,
        })
        .expect("a checkout session was created");

    assert_eq!(
        input.metadata.get("payment_type").map(String::as_str),
        Some("event_fee"),
    );
    assert_eq!(
        input.metadata.get("event_id").map(String::as_str),
        Some(event.id.to_string().as_str()),
    );
    let expires_at = input.expires_at.expect("an abandoned seat must be bounded");
    let horizon = expires_at - Utc::now().timestamp();
    assert!(
        horizon > 0 && horizon <= 3600,
        "expiry should be within 60 minutes, got {horizon}s",
    );

    // The payment row names the event, so the receipt does too.
    let payment = h
        .payment_repo
        .find_event_fee_payment(event.id, m.id)
        .await
        .unwrap()
        .unwrap();
    assert!(payment.description.contains("Lockpicking 101"));
    assert_eq!(payment.kind, PaymentKind::EventFee { event_id: event.id });
}
