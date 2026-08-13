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
        AttendanceStatus, Attendee, CreateMemberRequest, Event, EventType, EventVisibility, Member,
        MemberStatus, Payer, Payment, PaymentKind, PaymentMethod, PaymentStatus, StripeRef,
    },
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
use tokio::sync::Mutex;
use uuid::Uuid;

// ---------------------------------------------------------------------
// Recording EmailSender — the guest confirmation is the guest's only
// artifact of the registration, so it is asserted, not assumed.
// ---------------------------------------------------------------------

struct RecordingSender {
    sent: Mutex<Vec<coterie::email::EmailMessage>>,
}

impl RecordingSender {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            sent: Mutex::new(Vec::new()),
        })
    }
    async fn all(&self) -> Vec<coterie::email::EmailMessage> {
        self.sent.lock().await.clone()
    }
}

#[async_trait::async_trait]
impl coterie::email::EmailSender for RecordingSender {
    async fn send(&self, message: &coterie::email::EmailMessage) -> Result<(), AppError> {
        self.sent.lock().await.push(message.clone());
        Ok(())
    }
}

mod common;
use common::{build_charge, build_checkout_session, fresh_pool};

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
    emails: Arc<RecordingSender>,
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
    let emails = RecordingSender::new();
    let email_sender: Arc<dyn coterie::email::EmailSender> = emails.clone();
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
    let enrollment_repo: Arc<dyn coterie::repository::SeriesEnrollmentRepository> = Arc::new(
        coterie::repository::SqliteSeriesEnrollmentRepository::new(pool.clone()),
    );
    let dispatcher = WebhookDispatcher::new(
        gw,
        "whsec_test_dummy".to_string(),
        payment_repo.clone(),
        member_repo.clone(),
        event_repo.clone(),
        enrollment_repo.clone(),
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
        enrollment_repo.clone(),
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
        emails,
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
                guest_price_cents: 0,
                guest_registration_enabled: false,
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

    /// An event a non-member may register for: `Public` visibility plus
    /// the guest-registration flag, which is the whole test.
    async fn guest_event(
        &self,
        creator: Uuid,
        member_price_cents: i64,
        guest_price_cents: i64,
        max_attendees: Option<i32>,
    ) -> Event {
        let mut e = self.event(creator, member_price_cents, max_attendees).await;
        e.visibility = EventVisibility::Public;
        e.guest_price_cents = guest_price_cents;
        e.guest_registration_enabled = true;
        self.event_repo.update(e.id, e.clone()).await.unwrap()
    }

    /// Make `is_email_configured()` true, so the confirmation email is
    /// actually attempted (the default `log` mode is a dev no-op that
    /// skips silently).
    async fn configure_email(&self) {
        sqlx::query("UPDATE app_settings SET value = 'smtp' WHERE key = 'email.mode'")
            .execute(&self.pool)
            .await
            .unwrap();
        sqlx::query(
            "UPDATE app_settings SET value = 'smtp.example.com' WHERE key = 'email.smtp_host'",
        )
        .execute(&self.pool)
        .await
        .unwrap();
    }

    async fn guest_status(&self, event_id: Uuid, email: &str) -> Option<AttendanceStatus> {
        self.event_repo
            .attendance_status(
                event_id,
                &Attendee::Guest {
                    name: "ignored".to_string(),
                    email: email.to_string(),
                },
            )
            .await
            .unwrap()
    }

    async fn attendance_status(&self, event_id: Uuid, member_id: Uuid) -> Option<AttendanceStatus> {
        self.event_repo
            .attendance_status(event_id, &Attendee::Member(member_id))
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
    build_checkout_session(
        id,
        payment_intent,
        json!({
            "payment_type": "event_fee",
            "event_id": event_id.to_string(),
        }),
        3000,
    )
}

/// The same session as an abandoned one: only its id is read, and
/// flipping the Pending row to Failed is keyed off that.
fn expired_session(id: &str) -> stripe::CheckoutSession {
    build_checkout_session(id, None, json!({ "payment_type": "event_fee" }), 3000)
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
        .find_event_fee_payment(event.id, &Payer::Member(a.id))
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
        .find_event_fee_payment(event.id, &Payer::Member(m.id))
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
        .find_event_fee_payment(event.id, &Payer::Member(m.id))
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
    let dues_before = h
        .member_repo
        .find_by_id(m.id)
        .await
        .unwrap()
        .unwrap()
        .dues_paid_until;

    // A manual (at-the-door) event fee: refundable with no Stripe call.
    let payment = h
        .payment_service
        .record_manual(
            RecordManualPaymentInput {
                payer: Payer::Member(m.id),
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
        .register_attendance(event.id, &Attendee::Member(m.id))
        .await
        .unwrap();
    h.event_repo
        .link_payment(event.id, &Attendee::Member(m.id), payment.id)
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

    // A refunded membership payment retracts the dues it granted; an
    // event fee never granted any, so the window must not move.
    assert_eq!(
        h.member_repo
            .find_by_id(m.id)
            .await
            .unwrap()
            .unwrap()
            .dues_paid_until,
        dues_before,
        "refunding an event fee is not a dues retraction",
    );
    assert_eq!(h.audit_count("membership_dues_retracted").await, 0);
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
        .register_attendance(event.id, &Attendee::Member(m.id))
        .await
        .unwrap();
    h.event_repo
        .link_payment(event.id, &Attendee::Member(m.id), payment.id)
        .await
        .unwrap();

    h.dispatcher
        .dispatch_charge_refunded(build_charge("ch_oob", 3000, 3000, Some("pi_oob")))
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
                payer: Payer::Member(m.id),
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
        .register_attendance(event.id, &Attendee::Member(m.id))
        .await
        .unwrap();
    h.event_repo
        .link_payment(event.id, &Attendee::Member(m.id), payment.id)
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
        .register_attendance(event.id, &Attendee::Member(m.id))
        .await
        .unwrap();
    h.event_repo
        .link_payment(event.id, &Attendee::Member(m.id), payment.id)
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
                    payer: Payer::Member(member),
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
        .find_event_fee_payment(event.id, &Payer::Member(m.id))
        .await
        .unwrap()
        .unwrap();
    assert!(payment.description.contains("Lockpicking 101"));
    assert_eq!(payment.kind, PaymentKind::EventFee { event_id: event.id });
}

// =====================================================================
// Guest registration (a42). A guest seat is the same seat with a
// different payer — every test here checks that claim, so any drift
// toward a second state machine breaks something.
// =====================================================================

const GUEST: (&str, &str) = ("Ada Lovelace", "ada@example.com");

// 7.2 — the two questions stay two columns. "Non-members may not
// register" and "non-members attend free" must be different states in
// storage, because collapsing them is the bug this design exists to
// avoid.
#[tokio::test]
async fn disabled_guest_registration_is_distinguishable_from_a_zero_price() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;

    // Enabled at a zero price: a free workshop with a seat list.
    let free_public = h.guest_event(creator.id, 0, 0, Some(20)).await;
    // Disabled, also at a zero price: the weekly talk anyone walks into.
    let mut show_up = h.event(creator.id, 0, None).await;
    show_up.visibility = EventVisibility::Public;
    let show_up = h
        .event_repo
        .update(show_up.id, show_up.clone())
        .await
        .unwrap();

    assert!(free_public.publicly_registerable());
    assert!(!show_up.publicly_registerable());
    assert_eq!(free_public.guest_price_cents, 0);
    assert_eq!(show_up.guest_price_cents, 0);

    // Both are findable by equality on the price — the NULL-as-sentinel
    // failure the split avoids — and separable by the flag.
    let (zero_priced,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM events WHERE guest_price_cents = 0")
            .fetch_one(&h.pool)
            .await
            .unwrap();
    assert_eq!(
        zero_priced, 2,
        "a zero guest price is stored as zero, not NULL"
    );
    let (registerable,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events WHERE guest_registration_enabled = 1 AND visibility = 'Public'",
    )
    .fetch_one(&h.pool)
    .await
    .unwrap();
    assert_eq!(registerable, 1, "only the enabled one is registerable");

    // A registerable event carries a resolved URL; a show-up event does not.
    assert!(free_public
        .registration_url("https://coterie.example")
        .is_some());
    assert_eq!(show_up.registration_url("https://coterie.example"), None);
}

// 7.3 — the guest happy path end to end: seat held at Stripe, webhook
// confirms it, confirmation email carries the event AND the receipt.
#[tokio::test]
async fn guest_pays_and_the_webhook_confirms_the_seat_with_a_receipt_email() {
    let h = build_harness().await;
    h.configure_email().await;
    let creator = h.active_member("creator").await;
    let event = h.guest_event(creator.id, 0, 3000, Some(2)).await;

    let outcome = h
        .registration
        .register_guest(&event, GUEST.0, GUEST.1)
        .await
        .unwrap();
    assert!(matches!(outcome, RegistrationOutcome::Checkout { .. }));
    assert_eq!(
        h.guest_status(event.id, GUEST.1).await,
        Some(AttendanceStatus::PendingPayment),
        "the seat is held while the guest is at Stripe",
    );
    assert_eq!(h.event_repo.count_held_seats(event.id).await.unwrap(), 1);

    // The payment names a non-member payer and the event.
    let payment = h
        .payment_repo
        .find_event_fee_payment(
            event.id,
            &Payer::PublicDonor {
                name: GUEST.0.to_string(),
                email: GUEST.1.to_string(),
            },
        )
        .await
        .unwrap()
        .expect("a guest event-fee payment exists");
    assert_eq!(payment.status, PaymentStatus::Pending);
    assert_eq!(payment.member_id(), None);
    assert_eq!(
        payment.payer,
        Payer::PublicDonor {
            name: GUEST.0.to_string(),
            email: GUEST.1.to_string()
        },
    );
    assert_eq!(payment.kind, PaymentKind::EventFee { event_id: event.id });

    // Only the webhook confirms the seat.
    let session_id = payment.external_id.as_ref().unwrap().as_str().to_string();
    h.dispatcher
        .dispatch_checkout_session_completed(
            event_fee_session(&session_id, event.id, Some("pi_guest_1")),
            &h.billing,
        )
        .await
        .unwrap();

    assert_eq!(
        h.guest_status(event.id, GUEST.1).await,
        Some(AttendanceStatus::Registered),
    );
    let emails = h.emails.all().await;
    let confirmation = emails
        .iter()
        .find(|m| m.to == GUEST.1)
        .expect("the guest is emailed their confirmation");
    assert!(confirmation.subject.contains("Lockpicking 101"));
    assert!(confirmation.text_body.contains("Lockpicking 101"));
    assert!(
        confirmation.text_body.contains("$30.00"),
        "a paid registration's confirmation carries the amount paid: {}",
        confirmation.text_body,
    );
    assert_eq!(h.audit_count("event_registration_paid").await, 1);
}

// 3.1b / 7.2b — a free guest registration confirms immediately, writes
// no payment row, and still counts against capacity.
#[tokio::test]
async fn free_guest_registration_confirms_without_any_payment_machinery() {
    let h = build_harness().await;
    h.configure_email().await;
    let creator = h.active_member("creator").await;
    let event = h.guest_event(creator.id, 0, 0, Some(1)).await;

    let outcome = h
        .registration
        .register_guest(&event, GUEST.0, GUEST.1)
        .await
        .unwrap();
    assert_eq!(outcome, RegistrationOutcome::Registered);
    assert_eq!(
        h.guest_status(event.id, GUEST.1).await,
        Some(AttendanceStatus::Registered),
    );
    assert_eq!(h.payment_count().await, 0, "no payment row for a free seat");
    assert_eq!(
        h.fake.count_where(|c| matches!(
            c,
            coterie::payments::fake_gateway::FakeCall::CreateCheckoutSession(_)
        )),
        0,
        "no Checkout session for a free seat",
    );
    assert_eq!(
        h.event_repo.count_held_seats(event.id).await.unwrap(),
        1,
        "a free seat still occupies capacity",
    );

    // Re-submitting returns the existing seat rather than seating them twice.
    assert_eq!(
        h.registration
            .register_guest(&event, GUEST.0, GUEST.1)
            .await
            .unwrap(),
        RegistrationOutcome::Registered,
    );
    assert_eq!(h.event_repo.roster(event.id).await.unwrap().len(), 1);

    // The one-seat event is now full for everybody else.
    let other = h.active_member("other").await;
    assert!(
        h.registration.register(&other, &event).await.is_ok(),
        "free member RSVP is unchanged"
    );
}

// The free confirmation email confirms the seat and carries no receipt —
// a zero-amount receipt is noise.
#[tokio::test]
async fn free_guest_confirmation_carries_no_receipt() {
    let h = build_harness().await;
    h.configure_email().await;
    let creator = h.active_member("creator").await;
    let event = h.guest_event(creator.id, 0, 0, None).await;

    // The free path is confirmed by the public handler, which is what
    // sends the email; here we call the same dispatch the handler does.
    h.registration
        .register_guest(&event, GUEST.0, GUEST.1)
        .await
        .unwrap();
    coterie::service::billing_service::notifications::dispatch_guest_event_confirmation(
        &(h.emails.clone() as Arc<dyn coterie::email::EmailSender>),
        &coterie::service::settings_service::SettingsService::new(
            h.pool.clone(),
            Arc::new(SecretCrypto::new("test-secret-please-ignore")),
        ),
        GUEST.1,
        GUEST.0,
        &event,
        None,
    )
    .await;

    let emails = h.emails.all().await;
    let confirmation = emails.iter().find(|m| m.to == GUEST.1).expect("email sent");
    assert!(confirmation.text_body.contains("Lockpicking 101"));
    assert!(
        !confirmation.text_body.contains("Amount paid"),
        "a free registration gets no receipt: {}",
        confirmation.text_body,
    );
}

// 7.4 — abandonment frees a guest seat through a41's expiry path, with
// no guest-specific code involved.
#[tokio::test]
async fn abandoned_guest_checkout_frees_the_seat() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let event = h.guest_event(creator.id, 0, 3000, Some(1)).await;

    h.registration
        .register_guest(&event, GUEST.0, GUEST.1)
        .await
        .unwrap();
    assert_eq!(h.event_repo.count_held_seats(event.id).await.unwrap(), 1);

    let pending = h
        .payment_repo
        .find_event_fee_payment(
            event.id,
            &Payer::PublicDonor {
                name: GUEST.0.to_string(),
                email: GUEST.1.to_string(),
            },
        )
        .await
        .unwrap()
        .unwrap();
    let session_id = pending.external_id.as_ref().unwrap().as_str().to_string();
    h.dispatcher
        .dispatch_expired_session(expired_session(&session_id))
        .await
        .unwrap();

    assert_eq!(
        h.event_repo.count_held_seats(event.id).await.unwrap(),
        0,
        "the payment leaving Pending is what frees the seat",
    );
    // And the next guest can take it.
    assert!(matches!(
        h.registration
            .register_guest(&event, "Bob", "bob@example.com")
            .await,
        Ok(RegistrationOutcome::Checkout { .. }),
    ));
}

// 7.5 — one capacity, two kinds of seat. The count is row-based, so it
// needed no change; this proves it.
#[tokio::test]
async fn guest_and_member_seats_compete_for_the_same_capacity() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let event = h.guest_event(creator.id, 2000, 3000, Some(1)).await;
    let member = h.active_member("m").await;

    h.registration
        .register_guest(&event, GUEST.0, GUEST.1)
        .await
        .unwrap();

    let err = h.registration.register(&member, &event).await;
    assert!(
        matches!(&err, Err(AppError::BadRequest(msg)) if msg.contains("full")),
        "a guest's held seat closes the event for a member: {err:?}",
    );

    // And the other way around: free the guest seat, seat the member,
    // then the next guest is refused.
    h.event_repo
        .release_seat(
            event.id,
            &Attendee::Guest {
                name: GUEST.0.to_string(),
                email: GUEST.1.to_string(),
            },
        )
        .await
        .unwrap();
    h.registration.register(&member, &event).await.unwrap();
    let err = h
        .registration
        .register_guest(&event, "Bob", "bob@example.com")
        .await;
    assert!(
        matches!(&err, Err(AppError::BadRequest(msg)) if msg.contains("full")),
        "a member's held seat closes the event for a guest: {err:?}",
    );
}

// 7.6 — the UNIQUE(event_id, guest_email) constraint, not just the
// service guard: two concurrent submissions of one email yield one seat.
#[tokio::test]
async fn concurrent_duplicate_guest_email_yields_exactly_one_seat() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let event = h.guest_event(creator.id, 0, 3000, Some(5)).await;

    let (svc1, svc2) = (h.registration.clone(), h.registration.clone());
    let (e1, e2) = (event.clone(), event.clone());
    let t1 = tokio::spawn(async move { svc1.register_guest(&e1, GUEST.0, GUEST.1).await });
    let t2 = tokio::spawn(async move { svc2.register_guest(&e2, GUEST.0, GUEST.1).await });
    let _ = (t1.await.unwrap(), t2.await.unwrap());

    let roster = h.event_repo.roster(event.id).await.unwrap();
    assert_eq!(roster.len(), 1, "exactly one attendance row: {roster:?}");
    let (rows,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM event_attendance WHERE event_id = ? AND guest_email = ?",
    )
    .bind(event.id.to_string())
    .bind(GUEST.1)
    .fetch_one(&h.pool)
    .await
    .unwrap();
    assert_eq!(rows, 1, "the DB constraint is the guarantee, not the guard");
}

// 3.3 / 7.7 — a guest typing a member's address gets a guest seat. No
// row is written into that member's account, and the price bracket is
// the guest one. This is the deliberate divergence from /public/donate.
#[tokio::test]
async fn guest_using_a_member_email_does_not_touch_that_member() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let event = h.guest_event(creator.id, 1000, 3000, None).await;
    let member = h.active_member("victim").await;

    h.registration
        .register_guest(&event, "Not The Member", &member.email)
        .await
        .unwrap();

    // The member holds no seat and owns no payment.
    assert_eq!(
        h.attendance_status(event.id, member.id).await,
        None,
        "no attendance row in the member's account",
    );
    assert!(
        h.payment_repo
            .find_by_member(member.id)
            .await
            .unwrap()
            .is_empty(),
        "no payment row in the member's account",
    );

    // The seat that does exist is a guest seat at the guest price.
    let roster = h.event_repo.roster(event.id).await.unwrap();
    assert_eq!(roster.len(), 1);
    assert_eq!(
        roster[0].attendee,
        Attendee::Guest {
            name: "Not The Member".to_string(),
            email: member.email.clone(),
        },
    );
    let payment = h
        .payment_repo
        .find_event_fee_payment(
            event.id,
            &Payer::PublicDonor {
                name: "Not The Member".to_string(),
                email: member.email.clone(),
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(payment.member_id(), None);
    assert_eq!(
        payment.amount_cents, 3000,
        "charged the guest price, not the member's bracket",
    );
}

// 3.2 — the double-charge guard is keyed on (event, guest email).
#[tokio::test]
async fn a_paid_guest_resubmitting_is_not_charged_twice() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let event = h.guest_event(creator.id, 0, 3000, None).await;
    let payer = Payer::PublicDonor {
        name: GUEST.0.to_string(),
        email: GUEST.1.to_string(),
    };

    h.registration
        .register_guest(&event, GUEST.0, GUEST.1)
        .await
        .unwrap();
    let payment = h
        .payment_repo
        .find_event_fee_payment(event.id, &payer)
        .await
        .unwrap()
        .unwrap();
    let session_id = payment.external_id.as_ref().unwrap().as_str().to_string();
    h.dispatcher
        .dispatch_checkout_session_completed(
            event_fee_session(&session_id, event.id, Some("pi_guest_2")),
            &h.billing,
        )
        .await
        .unwrap();

    let sessions_before = h.fake.count_where(|c| {
        matches!(
            c,
            coterie::payments::fake_gateway::FakeCall::CreateCheckoutSession(_)
        )
    });
    let payments_before = h.payment_count().await;

    assert_eq!(
        h.registration
            .register_guest(&event, GUEST.0, GUEST.1)
            .await
            .unwrap(),
        RegistrationOutcome::Registered,
        "an already-paid guest is returned their registration",
    );
    assert_eq!(
        h.fake.count_where(|c| matches!(
            c,
            coterie::payments::fake_gateway::FakeCall::CreateCheckoutSession(_)
        )),
        sessions_before,
        "no second Checkout session",
    );
    assert_eq!(
        h.payment_count().await,
        payments_before,
        "no second payment row"
    );
}

// 7.9 — refunding a guest's fee releases the seat, by the same
// payment-keyed path a member's refund takes.
#[tokio::test]
async fn guest_refund_releases_the_seat() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let admin = h.active_member("admin").await;
    let event = h.guest_event(creator.id, 0, 3000, Some(1)).await;
    let payer = Payer::PublicDonor {
        name: GUEST.0.to_string(),
        email: GUEST.1.to_string(),
    };

    h.registration
        .register_guest(&event, GUEST.0, GUEST.1)
        .await
        .unwrap();
    let payment = h
        .payment_repo
        .find_event_fee_payment(event.id, &payer)
        .await
        .unwrap()
        .unwrap();
    let session_id = payment.external_id.as_ref().unwrap().as_str().to_string();
    h.dispatcher
        .dispatch_checkout_session_completed(
            event_fee_session(&session_id, event.id, Some("pi_guest_3")),
            &h.billing,
        )
        .await
        .unwrap();
    assert_eq!(h.event_repo.count_held_seats(event.id).await.unwrap(), 1);

    h.payment_admin
        .refund(admin.id, payment.id, loopback())
        .await
        .expect("a guest's event fee is refundable like a member's");

    let after = h
        .payment_repo
        .find_by_id(payment.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.status, PaymentStatus::Refunded);
    assert_eq!(
        h.guest_status(event.id, GUEST.1).await,
        Some(AttendanceStatus::Cancelled),
    );
    assert_eq!(
        h.event_repo.count_held_seats(event.id).await.unwrap(),
        0,
        "the seat is available again",
    );
}

// 6.2 / 6.3 — the roster shows a guest with their supplied identity and
// distinguishes them from a member, and the operator actions work on the
// guest row with identical audit actions.
#[tokio::test]
async fn roster_shows_guests_and_admin_actions_work_on_them() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let admin = h.active_member("admin").await;
    let member = h.active_member("m").await;
    let event = h.guest_event(creator.id, 2000, 3000, None).await;

    h.event_repo
        .register_attendance(event.id, &Attendee::Member(member.id))
        .await
        .unwrap();
    h.registration
        .register_guest(&event, GUEST.0, GUEST.1)
        .await
        .unwrap();

    let roster = h.event_repo.roster(event.id).await.unwrap();
    assert_eq!(roster.len(), 2, "both kinds of seat are listed: {roster:?}");
    let guest_row = roster
        .iter()
        .find(|r| r.attendee.member_id().is_none())
        .expect("the guest row is on the roster");
    assert_eq!(guest_row.name, GUEST.0);
    assert_eq!(guest_row.email, GUEST.1);
    let member_row = roster
        .iter()
        .find(|r| r.attendee.member_id() == Some(member.id))
        .expect("the member row is on the roster");
    assert_eq!(member_row.email, member.email);

    // At-the-door for the guest: a Manual event fee at the GUEST price,
    // audited with the same action a member's would be.
    let guest = Attendee::Guest {
        name: GUEST.0.to_string(),
        email: GUEST.1.to_string(),
    };
    let at_the_door = h
        .payment_service
        .record_manual(
            RecordManualPaymentInput {
                payer: guest.as_payer(),
                amount_cents: event.guest_price_cents,
                kind: PaymentKind::EventFee { event_id: event.id },
                description: "Event registration — Lockpicking 101".to_string(),
                payment_method: PaymentMethod::Manual,
                membership_type_slug: None,
                actor_id: admin.id,
            },
            &h.billing,
        )
        .await
        .expect("a guest can pay at the door");
    assert_eq!(at_the_door.member_id(), None);
    assert_eq!(at_the_door.amount_cents, 3000);
    assert_eq!(h.audit_count("manual_event_fee").await, 1);

    h.event_repo
        .register_attendance(event.id, &guest)
        .await
        .unwrap();
    h.event_repo
        .link_payment(event.id, &guest, at_the_door.id)
        .await
        .unwrap();
    assert_eq!(
        h.guest_status(event.id, GUEST.1).await,
        Some(AttendanceStatus::Registered),
    );

    // Comping a guest: $0 Waived, same audit action as a comped member.
    let other_guest = Attendee::Guest {
        name: "Bob".to_string(),
        email: "bob@example.com".to_string(),
    };
    h.payment_service
        .record_manual(
            RecordManualPaymentInput {
                payer: other_guest.as_payer(),
                amount_cents: 0,
                kind: PaymentKind::EventFee { event_id: event.id },
                description: "Event registration — Lockpicking 101".to_string(),
                payment_method: PaymentMethod::Waived,
                membership_type_slug: None,
                actor_id: admin.id,
            },
            &h.billing,
        )
        .await
        .expect("a guest seat can be comped");
    assert_eq!(h.audit_count("waive_event_fee").await, 1);
}

// A guest identity is bounded and validated before it can reach a seat.
#[tokio::test]
async fn guest_registration_rejects_unusable_identities() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let event = h.guest_event(creator.id, 0, 3000, None).await;

    for (name, email) in [("Ada", "no-at-sign"), ("", "ada@example.com"), ("Ada", "")] {
        assert!(
            h.registration
                .register_guest(&event, name, email)
                .await
                .is_err(),
            "name={name:?} email={email:?} should be rejected",
        );
    }
    assert_eq!(
        h.payment_count().await,
        0,
        "a rejected identity writes nothing"
    );
    assert!(h.event_repo.roster(event.id).await.unwrap().is_empty());
}

// The roster's "Release seat" control issues no refund, so it must
// refuse a seat whose fee is already Completed. That pairing is
// reachable: the completion webhook flips the payment, then fails to
// confirm the seat (an unlinked row, or a `confirm_seat` error whose
// Stripe retry short-circuits on the already-Completed payment). The
// row stays PendingPayment, which is what offers this control.
#[tokio::test]
async fn releasing_a_seat_refuses_when_the_attendee_already_paid() {
    use axum::{extract::State, response::IntoResponse, Extension, Form};
    use coterie::api::middleware::auth::CurrentUser;
    use coterie::web::portal::admin::events::{admin_roster_release_seat, RosterMemberForm};

    let h = build_harness().await;
    let admin = h.active_member("admin").await;
    let event = h.event(admin.id, 3000, None).await;
    let paid = h.active_member("paid").await;
    let abandoned = h.active_member("abandoned").await;

    for m in [&paid, &abandoned] {
        h.registration.register(m, &event).await.unwrap();
    }

    // The money moved, but the seat was never confirmed.
    let payment = h
        .payment_repo
        .find_event_fee_payment(event.id, &Payer::Member(paid.id))
        .await
        .unwrap()
        .unwrap();
    assert!(h
        .payment_repo
        .complete_pending_payment(payment.id, "pi_stuck")
        .await
        .unwrap());
    // ...and the link never landed, so `confirm_seat` matched nothing.
    sqlx::query("UPDATE event_attendance SET payment_id = NULL WHERE event_id = ?")
        .bind(event.id.to_string())
        .execute(&h.pool)
        .await
        .unwrap();
    assert_eq!(
        h.attendance_status(event.id, paid.id).await,
        Some(AttendanceStatus::PendingPayment),
    );

    let audit = Arc::new(AuditService::new(h.pool.clone()));
    let release = |member_id: Uuid| {
        admin_roster_release_seat(
            State(h.event_repo.clone()),
            State(h.payment_repo.clone()),
            State(audit.clone()),
            Extension(CurrentUser {
                member: admin.clone(),
            }),
            axum::extract::Path(event.id.to_string()),
            Form(RosterMemberForm {
                member_id: member_id.to_string(),
                guest_name: String::new(),
                guest_email: String::new(),
                csrf_token: String::new(),
            }),
        )
    };

    let body = axum::body::to_bytes(
        release(paid.id).await.into_response().into_body(),
        usize::MAX,
    )
    .await
    .unwrap();
    let body = String::from_utf8_lossy(&body);
    assert!(
        body.contains("already paid") && body.contains("refund"),
        "the admin is sent to the refund route: {body}",
    );
    assert_eq!(
        h.attendance_status(event.id, paid.id).await,
        Some(AttendanceStatus::PendingPayment),
        "a paid seat is NOT deleted by a control that issues no refund",
    );
    assert_eq!(
        h.payment_repo
            .find_by_id(payment.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        PaymentStatus::Completed,
        "and the money is left where it is, for the refund route to return",
    );

    // The control it exists for still works: an unpaid held seat goes.
    let _ = release(abandoned.id).await;
    assert_eq!(
        h.attendance_status(event.id, abandoned.id).await,
        None,
        "a seat nobody paid for is still released",
    );
}
