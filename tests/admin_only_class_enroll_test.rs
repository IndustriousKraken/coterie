//! `AdminOnly` at series scope: the member class-enroll endpoint.
//!
//! `POST /portal/api/series/:id/enroll` is the class-scope sibling of the
//! RSVP endpoint, and reachable by posting a series id directly. A series
//! row carries no visibility of its own, so the rule is resolved against
//! an occurrence — a non-admin who holds an `AdminOnly` series id must get
//! the same "class not found" answer an unknown id gives, with no
//! enrollment, no seat, no payment, and no Checkout session naming the
//! class they were not meant to know about.
//!
//! Run with: cargo test --features test-utils --test admin_only_class_enroll_test

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use chrono::{Duration, Utc};
use coterie::{
    api::state::AppState,
    domain::{
        AttendanceStatus, Attendee, CreateMemberRequest, Event, EventSeries, EventType,
        EventVisibility, Member, MemberStatus, Payer, UpdateMemberRequest,
    },
    payments::{
        fake_gateway::{FakeCall, FakeStripeGateway},
        gateway::StripeGateway,
        StripeClient, StripeHandle,
    },
    repository::{
        EventRepository, EventSeriesRepository, PaymentRepository, SeriesEnrollmentRepository,
        SqliteEventRepository, SqliteEventSeriesRepository, SqlitePaymentRepository,
        SqliteSeriesEnrollmentRepository,
    },
};
use sqlx::SqlitePool;
use tower::ServiceExt;
use uuid::Uuid;

mod common;
use common::{build_app_state_custom, fresh_pool};

const HIDDEN_TITLE: &str = "Board Revocation Rehearsal";
const PASS_CENTS: i64 = 12_000;

struct Harness {
    pool: SqlitePool,
    state: AppState,
    app: Router,
    fake: Arc<FakeStripeGateway>,
}

/// The portal router with a fake-gateway Stripe surface, so the paid
/// enroll path can actually mint a session when it is allowed to.
///
/// Built through `create_web_routes` alone (like the other portal tests):
/// the top-level CSRF layer is not what's under test here, and the
/// per-router auth gate that IS relevant lives inside the portal router.
async fn build() -> Harness {
    let pool = fresh_pool().await;
    let fake = Arc::new(FakeStripeGateway::new());
    let gw: Arc<dyn StripeGateway> = fake.clone();
    let client = Arc::new(StripeClient::with_gateway(
        gw,
        Arc::new(SqlitePaymentRepository::new(pool.clone())),
        Arc::new(coterie::repository::SqliteMemberRepository::new(
            pool.clone(),
        )),
    ));
    let handle = Arc::new(StripeHandle::preloaded(Some(client), None));
    let state = build_app_state_custom(pool.clone(), Some(handle), None).await;
    let app = coterie::web::create_web_routes(state.clone());
    Harness {
        pool,
        state,
        app,
        fake,
    }
}

impl Harness {
    /// An Active member (optionally an admin) plus a live session token.
    async fn member(&self, tag: &str, is_admin: bool) -> (Member, String) {
        let suffix = Uuid::new_v4();
        let member = self
            .state
            .service_context
            .member_repo
            .create(CreateMemberRequest {
                email: format!("{tag}-{suffix}@example.com"),
                username: format!("u_{}", suffix.simple()),
                full_name: format!("Member {tag}"),
                password: "p4ssword_long_enough".into(),
                membership_type_id: None,
                ..Default::default()
            })
            .await
            .expect("create member");
        let member = self
            .state
            .service_context
            .member_repo
            .update(
                member.id,
                UpdateMemberRequest {
                    status: Some(MemberStatus::Active),
                    ..Default::default()
                },
            )
            .await
            .expect("activate member");
        let member = if is_admin {
            self.state
                .service_context
                .member_repo
                .set_admin(member.id, true)
                .await
                .expect("set admin")
        } else {
            member
        };
        assert_eq!(member.status, MemberStatus::Active);
        let (_, token) = self
            .state
            .service_context
            .auth_service
            .create_session(member.id, 24)
            .await
            .expect("create session");
        (member, token)
    }

    /// A bounded, priced class with two future occurrences at `visibility`.
    async fn class(&self, creator: Uuid, visibility: EventVisibility) -> EventSeries {
        let now = Utc::now();
        let series_repo = SqliteEventSeriesRepository::new(self.pool.clone());
        let series = series_repo
            .create(EventSeries {
                id: Uuid::new_v4(),
                rule_kind: "weekly_by_day".to_string(),
                rule_json: r#"{"kind":"weekly_by_day","interval":1,"weekdays":["tue"]}"#
                    .to_string(),
                until_date: Some(now + Duration::days(21)),
                materialized_through: now + Duration::days(14),
                member_price_cents: PASS_CENTS,
                guest_price_cents: 0,
                guest_registration_enabled: false,
                max_enrollments: None,
                created_by: creator,
                created_at: now,
                updated_at: now,
            })
            .await
            .expect("create series");

        let event_repo = SqliteEventRepository::new(self.pool.clone());
        for (idx, offset) in [7i64, 14].iter().enumerate() {
            event_repo
                .create(Event {
                    id: Uuid::new_v4(),
                    title: HIDDEN_TITLE.to_string(),
                    description: "Two Tuesdays".to_string(),
                    event_type: EventType::Workshop,
                    event_type_id: None,
                    visibility: visibility.clone(),
                    start_time: now + Duration::days(*offset),
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
                    created_at: now,
                    updated_at: now,
                    series_id: Some(series.id),
                    occurrence_index: Some((idx + 1) as i32),
                })
                .await
                .expect("create occurrence");
        }
        series
    }

    async fn enroll(&self, series_id: Uuid, session: &str) -> (StatusCode, Option<String>, String) {
        let resp = self
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/portal/api/series/{series_id}/enroll"))
                    .header(header::COOKIE, format!("session={session}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let redirect = resp
            .headers()
            .get("HX-Redirect")
            .map(|v| v.to_str().unwrap().to_string());
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, redirect, String::from_utf8_lossy(&body).to_string())
    }

    async fn occurrences(&self, series_id: Uuid) -> Vec<Event> {
        SqliteEventRepository::new(self.pool.clone())
            .list_series_occurrences(series_id)
            .await
            .unwrap()
    }

    async fn enrollment_status(
        &self,
        series_id: Uuid,
        member_id: Uuid,
    ) -> Option<AttendanceStatus> {
        SqliteSeriesEnrollmentRepository::new(self.pool.clone())
            .find(series_id, &Attendee::Member(member_id))
            .await
            .unwrap()
            .map(|e| e.status)
    }

    async fn pass_payment_exists(&self, series_id: Uuid, member_id: Uuid) -> bool {
        SqlitePaymentRepository::new(self.pool.clone())
            .find_series_pass_payment(series_id, &Payer::Member(member_id))
            .await
            .unwrap()
            .is_some()
    }

    async fn enrollment_row_count(&self) -> i64 {
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM series_enrollment")
            .fetch_one(&self.pool)
            .await
            .unwrap();
        n
    }

    async fn payment_row_count(&self) -> i64 {
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM payments")
            .fetch_one(&self.pool)
            .await
            .unwrap();
        n
    }

    fn checkout_sessions_created(&self) -> usize {
        self.fake
            .count_where(|c| matches!(c, FakeCall::CreateCheckoutSession(_)))
    }
}

/// The enroll fragment names the series id it was posted for, so two
/// refusals can only be compared with that id normalized away. Everything
/// else — markup, price, error text — must match byte for byte.
fn without_id(body: &str, id: Uuid) -> String {
    body.replace(&id.to_string(), "<series-id>")
}

// ---------------------------------------------------------------------
// A non-admin is refused, indistinguishably from an unknown id
// ---------------------------------------------------------------------

#[tokio::test]
async fn enroll_in_admin_only_series_is_refused_for_a_non_admin() {
    let h = build().await;
    let (admin, _) = h.member("creator", true).await;
    let (member, session) = h.member("m", false).await;
    let series = h.class(admin.id, EventVisibility::AdminOnly).await;

    let (status, redirect, body) = h.enroll(series.id, &session).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        redirect.is_none(),
        "a refusal must not send anyone to Stripe"
    );

    // Byte-identical (modulo the id the caller supplied) to the answer a
    // series that does not exist gets — otherwise a member can probe for
    // admin-only class ids.
    let unknown_id = Uuid::new_v4();
    let (unknown_status, _, unknown_body) = h.enroll(unknown_id, &session).await;
    assert_eq!(unknown_status, StatusCode::OK);
    assert_eq!(
        without_id(&body, series.id),
        without_id(&unknown_body, unknown_id),
        "the refusal must read exactly as an unknown series id does",
    );

    // 3.3: the title the level exists to hide appears nowhere.
    assert!(
        !body.contains(HIDDEN_TITLE),
        "the admin-only class title leaked into the refusal: {body}",
    );

    // 3.2: nothing was written on the way out.
    assert_eq!(h.enrollment_status(series.id, member.id).await, None);
    assert_eq!(
        h.enrollment_row_count().await,
        0,
        "no series_enrollment row"
    );
    assert!(!h.pass_payment_exists(series.id, member.id).await);
    assert_eq!(h.payment_row_count().await, 0, "no payments row");
    assert_eq!(h.checkout_sessions_created(), 0, "no Checkout session");
    for occ in h.occurrences(series.id).await {
        assert_eq!(
            SqliteEventRepository::new(h.pool.clone())
                .attendance_status(occ.id, &Attendee::Member(member.id))
                .await
                .unwrap(),
            None,
            "no seat may be written on an occurrence the member can't see",
        );
    }
}

// ---------------------------------------------------------------------
// An admin still enrolls, and an ordinary class is untouched
// ---------------------------------------------------------------------

#[tokio::test]
async fn enroll_in_admin_only_series_succeeds_for_an_admin() {
    let h = build().await;
    let (admin, session) = h.member("admin", true).await;
    let series = h.class(admin.id, EventVisibility::AdminOnly).await;

    let (status, redirect, body) = h.enroll(series.id, &session).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        redirect.is_some(),
        "a priced class sends the admin to Checkout, got body: {body}",
    );
    assert_eq!(
        h.enrollment_status(series.id, admin.id).await,
        Some(AttendanceStatus::PendingPayment),
        "the place is held until the completion webhook confirms payment",
    );
    assert!(h.pass_payment_exists(series.id, admin.id).await);
    assert_eq!(h.checkout_sessions_created(), 1);
}

#[tokio::test]
async fn enroll_in_members_only_series_still_succeeds() {
    let h = build().await;
    let (creator, _) = h.member("creator", true).await;
    let (member, session) = h.member("m", false).await;
    let series = h.class(creator.id, EventVisibility::MembersOnly).await;

    let (status, redirect, body) = h.enroll(series.id, &session).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        redirect.is_some(),
        "an ordinary member enrollment must still reach Checkout, got body: {body}",
    );
    assert_eq!(
        h.enrollment_status(series.id, member.id).await,
        Some(AttendanceStatus::PendingPayment),
    );
    assert!(h.pass_payment_exists(series.id, member.id).await);
    assert_eq!(h.checkout_sessions_created(), 1);
}
