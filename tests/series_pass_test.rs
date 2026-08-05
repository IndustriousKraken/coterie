//! Series passes: one payment buys a place in every remaining session of
//! a bounded recurring class.
//!
//! The invariants under test: a priced class is always bounded, a
//! confirmed pass materializes attendance for exactly the sessions still
//! to come, capacity is enforced once at series scope and never re-checked
//! per night, and money moves in one direction per event — a refund
//! releases future sessions, a cancelled night releases nothing.
//!
//! Run with: cargo test --features test-utils --test series_pass_test

use std::net::IpAddr;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use coterie::{
    api::state::{MoneyLimiter, RateLimiter},
    auth::SecretCrypto,
    domain::{
        AttendanceStatus, Attendee, CreateMemberRequest, Event, EventSeries, EventType,
        EventVisibility, Member, MemberStatus, Payer, Payment, PaymentKind, PaymentMethod,
        PaymentStatus, Recurrence, SeriesPassPricing, StripeRef, WeekdayCode,
    },
    error::AppError,
    integrations::{public_site::PublicSiteNotifier, IntegrationManager},
    payments::{
        fake_gateway::FakeStripeGateway, gateway::StripeGateway, StripeClient, StripeHandle,
        WebhookDispatcher,
    },
    repository::{
        EventRepository, EventSeriesRepository, MemberRepository, PaymentRepository,
        SeriesEnrollmentRepository, SqliteEventRepository, SqliteEventSeriesRepository,
        SqliteMemberRepository, SqlitePaymentRepository, SqliteSavedCardRepository,
        SqliteScheduledPaymentRepository, SqliteSeriesEnrollmentRepository,
    },
    service::{
        audit_service::AuditService,
        billing_service::BillingService,
        event_admin_service::EventAdminService,
        event_registration_service::RegistrationOutcome,
        membership_type_service::MembershipTypeService,
        payment_admin_service::{PaymentAdminService, RefundError},
        payment_service::{PaymentService, RecordManualPaymentInput},
        recurring_event_service::{RecurringEventService, DEFAULT_HORIZON},
        series_enrollment_service::SeriesEnrollmentService,
        settings_service::SettingsService,
    },
};
use serde_json::json;
use sqlx::SqlitePool;
use uuid::Uuid;

mod common;
use common::{build_charge, build_checkout_session, fresh_pool};

// ---------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------

struct Harness {
    pool: SqlitePool,
    event_repo: Arc<dyn EventRepository>,
    series_repo: Arc<dyn EventSeriesRepository>,
    enrollment_repo: Arc<dyn SeriesEnrollmentRepository>,
    payment_repo: Arc<dyn PaymentRepository>,
    member_repo: Arc<dyn MemberRepository>,
    enrollment: Arc<SeriesEnrollmentService>,
    recurring: Arc<RecurringEventService>,
    event_admin: EventAdminService,
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
    let series_repo: Arc<dyn EventSeriesRepository> =
        Arc::new(SqliteEventSeriesRepository::new(pool.clone()));
    let enrollment_repo: Arc<dyn SeriesEnrollmentRepository> =
        Arc::new(SqliteSeriesEnrollmentRepository::new(pool.clone()));
    let audit_service = Arc::new(AuditService::new(pool.clone()));
    let integrations = Arc::new(IntegrationManager::new());
    let mt_service = Arc::new(MembershipTypeService::new(Arc::new(
        coterie::repository::SqliteMembershipTypeRepository::new(pool.clone()),
    )));
    let crypto = Arc::new(SecretCrypto::new("test-secret-please-ignore"));
    let settings = Arc::new(SettingsService::new(pool.clone(), crypto));
    let email_sender: Arc<dyn coterie::email::EmailSender> = Arc::new(
        coterie::email::LogSender::new("test@example.com".to_string(), "Test".to_string()),
    );
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
        audit_service.clone(),
    );

    let recurring = Arc::new(RecurringEventService::new(
        event_repo.clone(),
        series_repo.clone(),
        pool.clone(),
    ));

    let event_admin = EventAdminService::new(
        event_repo.clone(),
        series_repo.clone(),
        recurring.clone(),
        audit_service,
        integrations,
        Arc::new(PublicSiteNotifier::new(Arc::new(SettingsService::new(
            pool.clone(),
            Arc::new(SecretCrypto::new("test-secret-please-ignore")),
        )))),
    );

    let enrollment = Arc::new(SeriesEnrollmentService::new(
        event_repo.clone(),
        enrollment_repo.clone(),
        payment_repo.clone(),
        stripe_handle,
        "http://localhost:3000".to_string(),
    ));

    Harness {
        pool,
        event_repo,
        series_repo,
        enrollment_repo,
        payment_repo,
        member_repo,
        enrollment,
        recurring,
        event_admin,
        dispatcher,
        payment_admin,
        payment_service,
        billing,
        fake,
    }
}

impl Harness {
    /// An Active member — the enrollment rule only admits Active/Honorary.
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

    /// A class with one occurrence per entry in `session_offsets` (days
    /// relative to now — negative for a session that already happened).
    /// `until_date` is set past the last session, so a priced class is
    /// bounded, which is what makes it sellable.
    async fn class(
        &self,
        creator: Uuid,
        pricing: SeriesPassPricing,
        session_offsets: &[i64],
    ) -> EventSeries {
        let now = Utc::now();
        let last = session_offsets.iter().copied().max().unwrap_or(0);
        self.class_with_until(
            creator,
            pricing,
            session_offsets,
            Some(now + Duration::days(last + 1)),
        )
        .await
    }

    async fn class_with_until(
        &self,
        creator: Uuid,
        pricing: SeriesPassPricing,
        session_offsets: &[i64],
        until_date: Option<DateTime<Utc>>,
    ) -> EventSeries {
        let now = Utc::now();
        let series = self
            .series_repo
            .create(EventSeries {
                id: Uuid::new_v4(),
                rule_kind: "weekly_by_day".to_string(),
                rule_json: r#"{"kind":"weekly_by_day","interval":1,"weekdays":["tue"]}"#
                    .to_string(),
                until_date,
                materialized_through: now
                    + Duration::days(session_offsets.iter().copied().max().unwrap_or(0)),
                member_price_cents: pricing.member_price_cents,
                guest_price_cents: pricing.guest_price_cents,
                guest_registration_enabled: pricing.guest_registration_enabled,
                max_enrollments: pricing.max_enrollments,
                created_by: creator,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        for (idx, offset) in session_offsets.iter().enumerate() {
            self.occurrence(&series, creator, *offset, (idx + 1) as i32, None)
                .await;
        }
        series
    }

    async fn occurrence(
        &self,
        series: &EventSeries,
        creator: Uuid,
        day_offset: i64,
        index: i32,
        max_attendees: Option<i32>,
    ) -> Event {
        let now = Utc::now();
        self.event_repo
            .create(Event {
                id: Uuid::new_v4(),
                title: "Intro to Lockpicking".to_string(),
                description: "Six Tuesdays".to_string(),
                event_type: EventType::Workshop,
                event_type_id: None,
                visibility: EventVisibility::MembersOnly,
                start_time: now + Duration::days(day_offset),
                end_time: None,
                timezone: "UTC".to_string(),
                location: None,
                max_attendees,
                rsvp_required: true,
                member_price_cents: 0,
                guest_price_cents: 0,
                guest_registration_enabled: false,
                image_url: None,
                created_by: creator,
                created_at: now,
                updated_at: now,
                series_id: Some(series.id),
                occurrence_index: Some(index),
            })
            .await
            .unwrap()
    }

    async fn occurrences(&self, series_id: Uuid) -> Vec<Event> {
        self.event_repo
            .list_series_occurrences(series_id)
            .await
            .unwrap()
    }

    async fn attendance(&self, event_id: Uuid, member_id: Uuid) -> Option<AttendanceStatus> {
        self.event_repo
            .attendance_status(event_id, &Attendee::Member(member_id))
            .await
            .unwrap()
    }

    /// The pending series-pass payment the enrollment claim just created.
    async fn pass_payment(&self, series_id: Uuid, member_id: Uuid) -> Payment {
        self.payment_repo
            .find_series_pass_payment(series_id, &Payer::Member(member_id))
            .await
            .unwrap()
            .expect("a series-pass payment")
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

    /// Enroll and drive the completion webhook, so the member ends up
    /// with a confirmed pass — the state most tests start from.
    async fn enroll_and_pay(&self, member: &Member, series: &EventSeries) -> Payment {
        let outcome = self
            .enrollment
            .enroll(member, series, "Intro to Lockpicking")
            .await
            .unwrap();
        assert!(matches!(outcome, RegistrationOutcome::Checkout { .. }));
        self.complete_pass(series.id, member.id).await
    }

    /// Drive the completion webhook for an already-held enrollment.
    /// Returns the settled payment.
    async fn complete_pass(&self, series_id: Uuid, member_id: Uuid) -> Payment {
        let payment = self.pass_payment(series_id, member_id).await;
        let session_id = self.session_id_of(&payment);
        // A distinct PaymentIntent per payment: `payments.stripe_payment_id`
        // is UNIQUE, and Stripe would never reuse one either.
        let pi = format!("pi_{}", payment.id.simple());
        self.dispatcher
            .dispatch_checkout_session_completed(
                series_pass_session(&session_id, series_id, Some(&pi)),
                &self.billing,
            )
            .await
            .unwrap();
        self.payment_repo
            .find_by_id(payment.id)
            .await
            .unwrap()
            .unwrap()
    }

    fn session_id_of(&self, payment: &Payment) -> String {
        match payment.external_id.as_ref() {
            Some(StripeRef::CheckoutSession(id)) => id.clone(),
            other => panic!("expected a checkout session ref, got {other:?}"),
        }
    }
}

fn loopback() -> IpAddr {
    IpAddr::from([127, 0, 0, 1])
}

/// A `checkout.session.completed` payload for a series-pass session.
fn series_pass_session(
    id: &str,
    series_id: Uuid,
    payment_intent: Option<&str>,
) -> stripe::CheckoutSession {
    build_checkout_session(
        id,
        payment_intent,
        json!({
            "payment_type": "series_pass",
            "series_id": series_id.to_string(),
        }),
        12000,
    )
}

fn paid(member_cents: i64) -> SeriesPassPricing {
    SeriesPassPricing {
        member_price_cents: member_cents,
        ..Default::default()
    }
}

/// Priced for members AND guests, with the public door open.
fn paid_for_all(cents: i64) -> SeriesPassPricing {
    SeriesPassPricing {
        member_price_cents: cents,
        guest_price_cents: cents,
        guest_registration_enabled: true,
        max_enrollments: None,
    }
}

// ---------------------------------------------------------------------
// 8.1 A priced class must be bounded
// ---------------------------------------------------------------------

#[tokio::test]
async fn pass_price_on_an_unbounded_series_is_rejected() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;

    // Through the create path: no series and no occurrences are written.
    let anchor = Utc::now() + Duration::days(3);
    let err = h
        .recurring
        .create_series_with_initial_materialization(
            Recurrence::WeeklyByDay {
                interval: 1,
                weekdays: vec![WeekdayCode::Tue],
            },
            template(creator.id, anchor),
            None, // <- open-ended
            paid(12_000),
            creator.id,
        )
        .await
        .map(|_| ())
        .unwrap_err();
    match err {
        AppError::BadRequest(msg) => assert!(
            msg.contains("end date"),
            "message should name the missing end date, got: {msg}",
        ),
        other => panic!("expected BadRequest, got {other:?}"),
    }
    let (series_rows,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM event_series")
        .fetch_one(&h.pool)
        .await
        .unwrap();
    assert_eq!(series_rows, 0, "nothing persisted");

    // And through enrollment, for a row written some other way: an
    // unbounded priced class can't be sold either.
    let unbounded = h
        .class_with_until(creator.id, paid(12_000), &[7], None)
        .await;
    let m = h.active_member("m").await;
    let err = h
        .enrollment
        .enroll(&m, &unbounded, "Intro to Lockpicking")
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::BadRequest(_)), "got {err:?}");
    assert_eq!(
        h.payment_count().await,
        0,
        "no payment for an unsellable class"
    );

    // A bounded class accepts the same price.
    let bounded = h.class(creator.id, paid(12_000), &[7, 14]).await;
    assert!(bounded.is_paid_class());
    assert_eq!(bounded.member_price_cents, 12_000);
}

/// Zero is a price, not an absence: it's accepted and stored as zero,
/// negatives and over-cap values are not.
#[tokio::test]
async fn pass_price_bounds_match_a_single_event() {
    use coterie::domain::{validate_pass_pricing, MAX_PAYMENT_CENTS};
    let bounded = Some(Utc::now() + Duration::days(30));

    // Zero is fine even on an open-ended series — free classes stay free.
    assert!(validate_pass_pricing(&SeriesPassPricing::default(), None).is_ok());
    assert!(validate_pass_pricing(&paid(0), None).is_ok());
    assert!(validate_pass_pricing(&paid(MAX_PAYMENT_CENTS), bounded).is_ok());
    assert!(validate_pass_pricing(&paid(-1), bounded).is_err());
    assert!(validate_pass_pricing(&paid(MAX_PAYMENT_CENTS + 1), bounded).is_err());
}

fn template(creator: Uuid, start: DateTime<Utc>) -> Event {
    Event {
        id: Uuid::new_v4(),
        title: "Intro to Lockpicking".to_string(),
        description: "Six Tuesdays".to_string(),
        event_type: EventType::Workshop,
        event_type_id: None,
        visibility: EventVisibility::MembersOnly,
        start_time: start,
        end_time: None,
        timezone: "UTC".to_string(),
        location: None,
        max_attendees: None,
        rsvp_required: true,
        member_price_cents: 0,
        guest_price_cents: 0,
        guest_registration_enabled: false,
        image_url: None,
        created_by: creator,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        series_id: None,
        occurrence_index: None,
    }
}

// ---------------------------------------------------------------------
// 8.2 A confirmed pass seats every FUTURE session and no past one
// ---------------------------------------------------------------------

#[tokio::test]
async fn confirmed_pass_seats_future_sessions_only() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    // Six sessions, two of which already started.
    let series = h
        .class(creator.id, paid(12_000), &[-14, -7, 7, 14, 21, 28])
        .await;
    let m = h.active_member("m").await;

    // Before the webhook the enrollment is held, not confirmed, and no
    // attendance exists — the browser's return proves nothing.
    h.enrollment
        .enroll(&m, &series, "Intro to Lockpicking")
        .await
        .unwrap();
    let held = h
        .enrollment_repo
        .find(series.id, &Attendee::Member(m.id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(held.status, AttendanceStatus::PendingPayment);
    for occ in h.occurrences(series.id).await {
        assert_eq!(h.attendance(occ.id, m.id).await, None);
    }

    let payment = h.complete_pass(series.id, m.id).await;
    assert_eq!(payment.status, PaymentStatus::Completed);
    assert_eq!(
        h.enrollment_repo
            .find(series.id, &Attendee::Member(m.id))
            .await
            .unwrap()
            .unwrap()
            .status,
        AttendanceStatus::Registered,
    );

    let now = Utc::now();
    let mut future = 0;
    for occ in h.occurrences(series.id).await {
        if occ.start_utc() > now {
            assert_eq!(
                h.attendance(occ.id, m.id).await,
                Some(AttendanceStatus::Registered),
                "future session {:?} should be on the roster",
                occ.occurrence_index,
            );
            future += 1;
        } else {
            assert_eq!(
                h.attendance(occ.id, m.id).await,
                None,
                "an already-started session must NOT be back-filled",
            );
        }
    }
    assert_eq!(future, 4, "four remaining sessions");
}

/// A Stripe redelivery of the same completion is a no-op: one payment,
/// one enrollment, and no duplicate audit row.
#[tokio::test]
async fn replayed_completion_webhook_changes_nothing() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let series = h.class(creator.id, paid(12_000), &[7, 14]).await;
    let m = h.active_member("m").await;

    h.enrollment
        .enroll(&m, &series, "Intro to Lockpicking")
        .await
        .unwrap();
    // The session id Stripe would redeliver with, captured before the
    // completion upgrades the stored ref to the PaymentIntent.
    let session_id = h.session_id_of(&h.pass_payment(series.id, m.id).await);
    let payment = h.complete_pass(series.id, m.id).await;
    let pi = format!("pi_{}", payment.id.simple());

    h.dispatcher
        .dispatch_checkout_session_completed(
            series_pass_session(&session_id, series.id, Some(&pi)),
            &h.billing,
        )
        .await
        .unwrap();
    // And again against the id the row now carries, so neither shape of
    // redelivery can double-confirm.
    h.dispatcher
        .dispatch_checkout_session_completed(
            series_pass_session(&session_id, series.id, Some(&pi)),
            &h.billing,
        )
        .await
        .unwrap();

    assert_eq!(h.payment_count().await, 1, "still exactly one payment");
    assert_eq!(
        h.audit_count("series_enrollment_paid").await,
        1,
        "the audit row is behind the flip, so a retry doesn't repeat it",
    );
}

// ---------------------------------------------------------------------
// 8.3 Roll-forward extends an active enrollment
// ---------------------------------------------------------------------

#[tokio::test]
async fn horizon_roll_forward_seats_enrollees_on_new_occurrences() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;

    // A real series, so the real materializer runs. Anchor 7 days out.
    //
    // Both bounds below are derived from DEFAULT_HORIZON rather than
    // written as absolute day counts. `create_series_with_initial_
    // materialization` fills up to `min(now + DEFAULT_HORIZON, until_date)`,
    // and `extend_horizon` caps its target at `until_date` — so if
    // `until_date` sits at or just past the initial horizon there is
    // nothing left for the roll-forward to materialize and `added` is 0.
    //
    // That is exactly how this test used to fail: `until_date` was
    // now + 365 days, one day past the 52-week horizon, leaving a
    // one-day extension window that contained a Tuesday only when CI
    // happened to run on a Monday. Giving the window a 60-day span keeps
    // several Tuesdays inside it on every weekday, and deriving it from
    // the constant means a change to DEFAULT_HORIZON can't silently
    // shrink it back to nothing.
    let anchor = Utc::now() + Duration::days(7);
    let created = h
        .recurring
        .create_series_with_initial_materialization(
            Recurrence::WeeklyByDay {
                interval: 1,
                weekdays: vec![WeekdayCode::Tue],
            },
            {
                // Anchor on a Tuesday so the weekly rule's first candidate
                // matches the template's start.
                let mut t = template(creator.id, anchor);
                t.start_time = next_tuesday(anchor);
                t
            },
            Some(Utc::now() + DEFAULT_HORIZON + Duration::days(90)),
            paid(12_000),
            creator.id,
        )
        .await
        .unwrap();
    let series = created.series;
    let before: Vec<Uuid> = h
        .occurrences(series.id)
        .await
        .iter()
        .map(|e| e.id)
        .collect();
    assert!(!before.is_empty());

    let m = h.active_member("m").await;
    h.enroll_and_pay(&m, &series).await;
    for id in &before {
        assert_eq!(
            h.attendance(*id, m.id).await,
            Some(AttendanceStatus::Registered)
        );
    }

    // Roll the horizon out past the initial window. Re-read the series so
    // `materialized_through` is current.
    let series = h.series_repo.find_by_id(series.id).await.unwrap().unwrap();
    let added = h
        .recurring
        .extend_horizon(&series, Utc::now() + DEFAULT_HORIZON + Duration::days(60))
        .await
        .unwrap();
    assert!(
        added > 0,
        "the roll-forward should materialize new sessions (window is \
         DEFAULT_HORIZON..+60d and must stay under until_date at +90d)"
    );

    for occ in h.occurrences(series.id).await {
        if before.contains(&occ.id) {
            continue;
        }
        assert_eq!(
            h.attendance(occ.id, m.id).await,
            Some(AttendanceStatus::Registered),
            "an enrollee must not silently vanish from a newly materialized session",
        );
    }
}

fn next_tuesday(from: DateTime<Utc>) -> DateTime<Utc> {
    use chrono::{Datelike, Weekday};
    let days = (Weekday::Tue.num_days_from_monday() as i64
        - from.weekday().num_days_from_monday() as i64)
        .rem_euclid(7);
    from + Duration::days(days)
}

// ---------------------------------------------------------------------
// 8.4 Class capacity is race-safe
// ---------------------------------------------------------------------

#[tokio::test]
async fn last_place_race_produces_exactly_one_winner() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let series = h
        .class(
            creator.id,
            SeriesPassPricing {
                member_price_cents: 12_000,
                max_enrollments: Some(1),
                ..Default::default()
            },
            &[7, 14],
        )
        .await;
    let a = h.active_member("a").await;
    let b = h.active_member("b").await;

    let (svc_a, svc_b) = (h.enrollment.clone(), h.enrollment.clone());
    let (s1, s2) = (series.clone(), series.clone());
    let h1 = tokio::spawn(async move { svc_a.enroll(&a, &s1, "Intro to Lockpicking").await });
    let h2 = tokio::spawn(async move { svc_b.enroll(&b, &s2, "Intro to Lockpicking").await });
    let (r1, r2) = (h1.await.unwrap(), h2.await.unwrap());

    let winners = [&r1, &r2].iter().filter(|r| r.is_ok()).count();
    assert_eq!(winners, 1, "exactly one claim: r1={r1:?} r2={r2:?}");
    assert_eq!(h.enrollment_repo.count_held(series.id).await.unwrap(), 1);
    assert_eq!(h.payment_count().await, 1, "the loser is charged nothing");
}

/// A full class rejects the next enrollment before any money is involved.
#[tokio::test]
async fn a_full_class_creates_no_payment_and_no_session() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let series = h
        .class(
            creator.id,
            SeriesPassPricing {
                member_price_cents: 12_000,
                max_enrollments: Some(1),
                ..Default::default()
            },
            &[7],
        )
        .await;
    let a = h.active_member("a").await;
    let b = h.active_member("b").await;

    h.enrollment
        .enroll(&a, &series, "Intro to Lockpicking")
        .await
        .unwrap();
    let before = h.payment_count().await;

    let err = h
        .enrollment
        .enroll(&b, &series, "Intro to Lockpicking")
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::BadRequest(_)), "got {err:?}");
    assert_eq!(
        h.payment_count().await,
        before,
        "no payment for a rejected click"
    );
    assert!(h
        .enrollment_repo
        .find(series.id, &Attendee::Member(b.id))
        .await
        .unwrap()
        .is_none());
}

/// Buying the same pass twice charges once.
#[tokio::test]
async fn a_second_enrollment_after_paying_charges_nothing() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let series = h.class(creator.id, paid(12_000), &[7, 14]).await;
    let m = h.active_member("m").await;

    h.enroll_and_pay(&m, &series).await;
    let before = h.payment_count().await;

    let outcome = h
        .enrollment
        .enroll(&m, &series, "Intro to Lockpicking")
        .await
        .unwrap();
    assert_eq!(outcome, RegistrationOutcome::Registered);
    assert_eq!(h.payment_count().await, before, "no second charge");
}

// ---------------------------------------------------------------------
// 8.5 A pass-holder is never bounced by a per-occurrence cap
// ---------------------------------------------------------------------

#[tokio::test]
async fn pass_holders_are_not_re_checked_against_max_attendees() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let series = h.class(creator.id, paid(12_000), &[]).await;
    // One session whose room holds ONE person, and three pass-holders.
    let tight = h.occurrence(&series, creator.id, 7, 1, Some(1)).await;

    let mut members = Vec::new();
    for tag in ["a", "b", "c"] {
        let m = h.active_member(tag).await;
        h.enroll_and_pay(&m, &series).await;
        members.push(m);
    }

    for m in &members {
        assert_eq!(
            h.attendance(tight.id, m.id).await,
            Some(AttendanceStatus::Registered),
            "the place was bought at series scope; the room cap must not take it back",
        );
    }
}

// ---------------------------------------------------------------------
// 8.6 Flat pricing: full price late, full refund mid-class
// ---------------------------------------------------------------------

#[tokio::test]
async fn late_enrollee_pays_full_price_and_a_mid_class_refund_returns_it_all() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    // Four of six sessions already held.
    let series = h
        .class(creator.id, paid(12_000), &[-28, -21, -14, -7, 7, 14])
        .await;
    let m = h.active_member("late").await;

    let payment = h.enroll_and_pay(&m, &series).await;
    assert_eq!(
        payment.amount_cents, 12_000,
        "no proration: a late enrollee pays the full pass price",
    );

    let outcome = h
        .payment_admin
        .refund(creator.id, payment.id, loopback())
        .await
        .unwrap();
    assert_eq!(
        outcome.amount_cents, 12_000,
        "no proration on the way out either",
    );
}

// ---------------------------------------------------------------------
// 8.7 A refund cancels the future and preserves the past
// ---------------------------------------------------------------------

#[tokio::test]
async fn refund_cancels_future_sessions_and_keeps_past_attendance() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let series = h.class(creator.id, paid(12_000), &[7, 14, 21, 28]).await;
    let m = h.active_member("m").await;

    let payment = h.enroll_and_pay(&m, &series).await;

    // Two sessions happen: move them into the past, keeping the seats.
    let occurrences = h.occurrences(series.id).await;
    for occ in occurrences.iter().take(2) {
        sqlx::query("UPDATE events SET start_time = ? WHERE id = ?")
            .bind((Utc::now() - Duration::days(3)).naive_utc())
            .bind(occ.id.to_string())
            .execute(&h.pool)
            .await
            .unwrap();
    }

    h.payment_admin
        .refund(creator.id, payment.id, loopback())
        .await
        .unwrap();

    assert_eq!(
        h.enrollment_repo
            .find(series.id, &Attendee::Member(m.id))
            .await
            .unwrap()
            .unwrap()
            .status,
        AttendanceStatus::Cancelled,
    );
    for occ in occurrences.iter().take(2) {
        assert_eq!(
            h.attendance(occ.id, m.id).await,
            Some(AttendanceStatus::Registered),
            "a session that already happened records who was there",
        );
    }
    for occ in occurrences.iter().skip(2) {
        assert_eq!(
            h.attendance(occ.id, m.id).await,
            Some(AttendanceStatus::Cancelled),
            "the remaining sessions go back to the class",
        );
    }
    // And the place is free again.
    assert_eq!(h.enrollment_repo.count_held(series.id).await.unwrap(), 0);
}

/// The same transitions when the refund is observed via the webhook
/// rather than issued through the admin route.
#[tokio::test]
async fn out_of_band_refund_webhook_cancels_the_enrollment() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let series = h.class(creator.id, paid(12_000), &[7, 14]).await;
    let m = h.active_member("m").await;

    let payment = h.enroll_and_pay(&m, &series).await;
    let pi = payment.external_id.as_ref().unwrap().as_str().to_string();
    h.dispatcher
        .dispatch_charge_refunded(build_charge("ch_1", 12_000, 12_000, Some(&pi)))
        .await
        .unwrap();

    assert_eq!(
        h.payment_repo
            .find_by_id(payment.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        PaymentStatus::Refunded,
    );
    assert_eq!(
        h.enrollment_repo
            .find(series.id, &Attendee::Member(m.id))
            .await
            .unwrap()
            .unwrap()
            .status,
        AttendanceStatus::Cancelled,
    );
    for occ in h.occurrences(series.id).await {
        assert_eq!(
            h.attendance(occ.id, m.id).await,
            Some(AttendanceStatus::Cancelled)
        );
    }
    assert_eq!(h.audit_count("series_enrollment_refunded").await, 1);
}

// ---------------------------------------------------------------------
// 8.8 Cancelling one occurrence is not a refund event
// ---------------------------------------------------------------------

#[tokio::test]
async fn cancelling_one_session_refunds_nobody_and_keeps_the_enrollment() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let series = h.class(creator.id, paid(12_000), &[7, 14, 21]).await;
    let m = h.active_member("m").await;
    let payment = h.enroll_and_pay(&m, &series).await;

    let occurrences = h.occurrences(series.id).await;
    let skipped = occurrences[1].clone();

    h.event_admin
        .cancel_event_occurrence(
            creator.id,
            series.id,
            skipped.occurrence_index.unwrap(),
            Some("snow day".to_string()),
        )
        .await
        .unwrap();

    // No money moved.
    assert_eq!(
        h.payment_repo
            .find_by_id(payment.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        PaymentStatus::Completed,
        "a holiday skip is not a partial cancellation of the product",
    );
    assert_eq!(
        h.fake.count_where(|c| matches!(
            c,
            coterie::payments::fake_gateway::FakeCall::CreateRefund(_)
        )),
        0,
        "no refund call",
    );
    // The enrollment stands.
    assert_eq!(
        h.enrollment_repo
            .find(series.id, &Attendee::Member(m.id))
            .await
            .unwrap()
            .unwrap()
            .status,
        AttendanceStatus::Registered,
    );
    // Only that session's attendance went — a35 hard-deletes the
    // occurrence, and `event_attendance` cascades with it.
    assert!(h.event_repo.find_by_id(skipped.id).await.unwrap().is_none());
    assert_eq!(h.attendance(skipped.id, m.id).await, None);
    for occ in [&occurrences[0], &occurrences[2]] {
        assert_eq!(
            h.attendance(occ.id, m.id).await,
            Some(AttendanceStatus::Registered),
            "every other session is untouched",
        );
    }
}

// ---------------------------------------------------------------------
// 8.9 Deleting a class refunds everybody first, and aborts on failure
// ---------------------------------------------------------------------

#[tokio::test]
async fn deleting_a_class_refunds_every_enrollee_first() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let series = h.class(creator.id, paid(12_000), &[7, 14]).await;

    let mut payments = Vec::new();
    for tag in ["a", "b", "c"] {
        let m = h.active_member(tag).await;
        let payment = h.enroll_and_pay(&m, &series).await;
        payments.push((m, payment));
    }

    let refunded = h
        .payment_admin
        .refund_all_series_passes(creator.id, series.id, loopback())
        .await
        .unwrap();
    assert_eq!(refunded, 3);

    for (m, p) in &payments {
        assert_eq!(
            h.payment_repo
                .find_by_id(p.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            PaymentStatus::Refunded,
        );
        assert_eq!(
            h.enrollment_repo
                .find(series.id, &Attendee::Member(m.id))
                .await
                .unwrap()
                .unwrap()
                .status,
            AttendanceStatus::Cancelled,
        );
    }
    assert_eq!(h.audit_count("refund_series_passes_bulk").await, 1);

    // Only after the refunds does the class go.
    h.series_repo.delete(series.id).await.unwrap();
    assert!(h.series_repo.find_by_id(series.id).await.unwrap().is_none());
    assert!(h.occurrences(series.id).await.is_empty());
}

#[tokio::test]
async fn a_failing_refund_aborts_the_class_delete() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let series = h.class(creator.id, paid(12_000), &[7, 14]).await;
    let m = h.active_member("m").await;

    // A Stripe-method pass whose refund Stripe will reject.
    let now = Utc::now();
    let payment = h
        .payment_repo
        .create(Payment {
            id: Uuid::new_v4(),
            payer: Payer::Member(m.id),
            amount_cents: 12_000,
            currency: "USD".to_string(),
            status: PaymentStatus::Completed,
            payment_method: PaymentMethod::Stripe,
            kind: PaymentKind::SeriesPass {
                series_id: series.id,
            },
            external_id: Some(StripeRef::PaymentIntent("pi_broken".to_string())),
            description: "Class pass — Intro to Lockpicking".to_string(),
            paid_at: Some(now),
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    h.enrollment_repo
        .register(series.id, &Attendee::Member(m.id))
        .await
        .unwrap();
    h.enrollment_repo
        .link_payment(series.id, &Attendee::Member(m.id), payment.id)
        .await
        .unwrap();

    h.fake
        .next_refund_err(AppError::External("card network down".to_string()));

    let err = h
        .payment_admin
        .refund_all_series_passes(creator.id, series.id, loopback())
        .await
        .unwrap_err();
    assert!(matches!(err, RefundError::StripeApiError(_)), "got {err:?}");

    // The caller must NOT delete: everything is exactly as it was.
    assert!(h.series_repo.find_by_id(series.id).await.unwrap().is_some());
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
        h.enrollment_repo
            .find(series.id, &Attendee::Member(m.id))
            .await
            .unwrap()
            .unwrap()
            .status,
        AttendanceStatus::Registered,
        "the class roster is intact",
    );
}

// ---------------------------------------------------------------------
// 8.10 Audit strings
// ---------------------------------------------------------------------

#[tokio::test]
async fn manual_and_waived_series_passes_audit_under_their_own_actions() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let series = h.class(creator.id, paid(12_000), &[7]).await;
    let event = h
        .event_repo
        .find_by_id(h.occurrences(series.id).await[0].id)
        .await
        .unwrap()
        .unwrap();

    let cases: [(PaymentMethod, PaymentKind, &str); 6] = [
        (
            PaymentMethod::Manual,
            PaymentKind::SeriesPass {
                series_id: series.id,
            },
            "manual_series_pass",
        ),
        (
            PaymentMethod::Waived,
            PaymentKind::SeriesPass {
                series_id: series.id,
            },
            "waive_series_pass",
        ),
        // The mappings that existed before a43 keep producing the same
        // strings — existing audit history stays meaningful.
        (
            PaymentMethod::Manual,
            PaymentKind::EventFee { event_id: event.id },
            "manual_event_fee",
        ),
        (
            PaymentMethod::Waived,
            PaymentKind::EventFee { event_id: event.id },
            "waive_event_fee",
        ),
        (
            PaymentMethod::Manual,
            PaymentKind::Membership,
            "manual_payment",
        ),
        (PaymentMethod::Waived, PaymentKind::Membership, "waive_dues"),
    ];

    for (method, kind, expected) in cases {
        let payer = h.active_member("payer").await;
        h.payment_service
            .record_manual(
                RecordManualPaymentInput {
                    payer: Payer::Member(payer.id),
                    amount_cents: if method == PaymentMethod::Waived {
                        0
                    } else {
                        12_000
                    },
                    kind,
                    description: "audit mapping".to_string(),
                    payment_method: method,
                    membership_type_slug: None,
                    actor_id: creator.id,
                },
                &h.billing,
            )
            .await
            .unwrap();
        assert_eq!(
            h.audit_count(expected).await,
            1,
            "expected exactly one `{expected}` audit row",
        );
    }
}

// ---------------------------------------------------------------------
// A finished class cannot be bought
//
// The floor under flat pricing. `seat_future_occurrences` seats nobody
// once every session has started, so a sale past that point is money for
// zero attendance — the failure paid events exist to prevent. The class
// page hides the form, but the endpoints are the trust boundary.
// ---------------------------------------------------------------------

impl Harness {
    async fn enrollment_row_count(&self) -> i64 {
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM series_enrollment")
            .fetch_one(&self.pool)
            .await
            .unwrap();
        n
    }

    fn checkout_sessions_created(&self) -> usize {
        self.fake.count_where(|c| {
            matches!(
                c,
                coterie::payments::fake_gateway::FakeCall::CreateCheckoutSession(_)
            )
        })
    }
}

#[tokio::test]
async fn enroll_guest_is_refused_when_every_session_has_started() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let series = h
        .class(creator.id, paid_for_all(12_000), &[-28, -21, -14, -7])
        .await;

    let err = h
        .enrollment
        .enroll_guest(
            &series,
            "Intro to Lockpicking",
            "Ada Lovelace",
            "ada@example.com",
        )
        .await
        .unwrap_err();

    assert!(matches!(err, AppError::BadRequest(_)), "got {err:?}");
    assert_eq!(h.enrollment_row_count().await, 0, "no enrollment row");
    assert_eq!(h.payment_count().await, 0, "no payment row");
    assert_eq!(h.checkout_sessions_created(), 0, "no Checkout session");
}

#[tokio::test]
async fn enroll_member_is_refused_when_every_session_has_started() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let series = h
        .class(creator.id, paid(12_000), &[-28, -21, -14, -7])
        .await;
    let m = h.active_member("late").await;

    let err = h
        .enrollment
        .enroll(&m, &series, "Intro to Lockpicking")
        .await
        .unwrap_err();

    assert!(matches!(err, AppError::BadRequest(_)), "got {err:?}");
    assert_eq!(h.enrollment_row_count().await, 0, "no enrollment row");
    assert_eq!(h.payment_count().await, 0, "no payment row");
    assert_eq!(h.checkout_sessions_created(), 0, "no Checkout session");
}

/// The floor is a floor, not proration: one session left is a full-price
/// sale.
#[tokio::test]
async fn enroll_succeeds_with_one_session_remaining() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let series = h
        .class(creator.id, paid(12_000), &[-35, -28, -21, -14, -7, 7])
        .await;
    let m = h.active_member("last-minute").await;

    let outcome = h
        .enrollment
        .enroll(&m, &series, "Intro to Lockpicking")
        .await
        .unwrap();
    assert!(
        matches!(outcome, RegistrationOutcome::Checkout { .. }),
        "got {outcome:?}",
    );

    let payment = h.pass_payment(series.id, m.id).await;
    assert_eq!(payment.amount_cents, 12_000, "still the full pass price");
    assert_eq!(
        h.enrollment_repo
            .find(series.id, &Attendee::Member(m.id))
            .await
            .unwrap()
            .unwrap()
            .status,
        AttendanceStatus::PendingPayment,
    );
}

/// The free short-circuit is behind the same guard — a free finished
/// class would hand out an enrollment that seats nobody.
#[tokio::test]
async fn free_finished_class_enrollment_is_refused() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let series = h
        .class_with_until(
            creator.id,
            SeriesPassPricing::default(),
            &[-28, -21, -14, -7],
            None,
        )
        .await;
    let m = h.active_member("m").await;

    let err = h
        .enrollment
        .enroll(&m, &series, "Intro to Lockpicking")
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::BadRequest(_)), "got {err:?}");

    let guest_err = h
        .enrollment
        .enroll_guest(
            &series,
            "Intro to Lockpicking",
            "Ada Lovelace",
            "ada@example.com",
        )
        .await
        .unwrap_err();
    assert!(
        matches!(guest_err, AppError::BadRequest(_)),
        "got {guest_err:?}",
    );

    assert_eq!(h.enrollment_row_count().await, 0, "no enrollment row");
    assert_eq!(h.payment_count().await, 0, "no payment row");
}

// ---------------------------------------------------------------------
// Free classes touch no payment machinery
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_free_class_enrolls_with_no_payment_and_no_checkout() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let series = h
        .class_with_until(creator.id, SeriesPassPricing::default(), &[7, 14], None)
        .await;
    let m = h.active_member("m").await;

    let outcome = h
        .enrollment
        .enroll(&m, &series, "Intro to Lockpicking")
        .await
        .unwrap();
    assert_eq!(outcome, RegistrationOutcome::Registered);
    assert_eq!(h.payment_count().await, 0, "no payment row");
    assert_eq!(
        h.fake.count_where(|c| matches!(
            c,
            coterie::payments::fake_gateway::FakeCall::CreateCheckoutSession(_)
        )),
        0,
        "no Checkout session",
    );
    for occ in h.occurrences(series.id).await {
        assert_eq!(
            h.attendance(occ.id, m.id).await,
            Some(AttendanceStatus::Registered),
        );
    }
}

// ---------------------------------------------------------------------
// Pricing columns default to zero, and rosters read one table
// ---------------------------------------------------------------------

/// The migration's `NOT NULL DEFAULT 0` is what keeps every series that
/// existed before passes free: a row written without the pricing columns
/// reads back as a free, uncapped class rather than a NULL-priced one.
#[tokio::test]
async fn a_series_written_without_pricing_columns_is_free() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let id = Uuid::new_v4();
    let now = Utc::now().naive_utc();

    sqlx::query(
        "INSERT INTO event_series \
             (id, rule_kind, rule_json, materialized_through, created_by, created_at, updated_at) \
         VALUES (?, 'weekly_by_day', '{}', ?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(now)
    .bind(creator.id.to_string())
    .bind(now)
    .bind(now)
    .execute(&h.pool)
    .await
    .unwrap();

    let series = h.series_repo.find_by_id(id).await.unwrap().unwrap();
    assert_eq!(series.member_price_cents, 0);
    assert_eq!(series.guest_price_cents, 0);
    assert!(!series.guest_registration_enabled);
    assert_eq!(series.max_enrollments, None);
    assert!(!series.is_paid_class(), "an untouched series stays free");
}

/// Comping somebody into a class records a `$0` `Waived` pass, confirms
/// the enrollment, and seats them — with no Stripe charge. And because
/// enrollment materializes attendance, the PER-OCCURRENCE roster shows
/// the pass-holder with no second read path.
#[tokio::test]
async fn a_comped_enrollment_charges_nothing_and_shows_on_each_night() {
    let h = build_harness().await;
    let creator = h.active_member("creator").await;
    let series = h.class(creator.id, paid(12_000), &[7, 14]).await;
    let m = h.active_member("comped").await;

    let payment = h
        .payment_service
        .record_manual(
            RecordManualPaymentInput {
                payer: Payer::Member(m.id),
                amount_cents: 0,
                kind: PaymentKind::SeriesPass {
                    series_id: series.id,
                },
                description: "Class pass — Intro to Lockpicking".to_string(),
                payment_method: PaymentMethod::Waived,
                membership_type_slug: None,
                actor_id: creator.id,
            },
            &h.billing,
        )
        .await
        .unwrap();
    h.enrollment_repo
        .register(series.id, &Attendee::Member(m.id))
        .await
        .unwrap();
    h.enrollment_repo
        .link_payment(series.id, &Attendee::Member(m.id), payment.id)
        .await
        .unwrap();
    coterie::service::series_enrollment_service::seat_future_occurrences(
        &*h.event_repo,
        series.id,
        &Attendee::Member(m.id),
        Some(payment.id),
    )
    .await
    .unwrap();

    assert_eq!(payment.amount_cents, 0);
    assert_eq!(payment.payment_method, PaymentMethod::Waived);
    assert_eq!(
        h.fake.count_where(|c| matches!(
            c,
            coterie::payments::fake_gateway::FakeCall::CreateCheckoutSession(_)
        )),
        0,
        "comping never reaches Stripe",
    );
    assert_eq!(h.audit_count("waive_series_pass").await, 1);

    // The class roster and every night's roster both know about them.
    let class_roster = h.enrollment_repo.roster(series.id).await.unwrap();
    assert_eq!(class_roster.len(), 1);
    assert_eq!(class_roster[0].attendee, Attendee::Member(m.id));
    assert_eq!(
        class_roster[0].payment_status,
        Some(PaymentStatus::Completed)
    );

    for occ in h.occurrences(series.id).await {
        let roster = h.event_repo.roster(occ.id).await.unwrap();
        assert!(
            roster.iter().any(|r| r.attendee == Attendee::Member(m.id)
                && r.status == AttendanceStatus::Registered),
            "the pass-holder belongs on occurrence {:?}'s roster",
            occ.occurrence_index,
        );
    }
}
