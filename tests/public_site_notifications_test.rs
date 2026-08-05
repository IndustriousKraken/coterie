//! Integration tests for the public-site change-notification change.
//!
//! Two halves, matching the two things the capability promises:
//!
//!   - **Dispatch** — every mutation that can change what
//!     `/public/events` or `/public/announcements` returns dispatches
//!     the matching `IntegrationEvent`, carrying no more than the item's
//!     own visibility already discloses, and nothing at all when the
//!     change is invisible to the public or the write failed.
//!   - **Notification** — what actually leaves the host: a signed,
//!     content-free message, bounded in time, inert when unconfigured,
//!     and reported to the admin on a withdrawal.
//!
//! Run: cargo test --features test-utils --test public_site_notifications_test

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Request, StatusCode},
    routing::post,
    Router,
};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use coterie::{
    domain::{
        AnnouncementType, Event, EventType, EventVisibility, Recurrence, UpdateSettingRequest,
        WeekdayCode,
    },
    error::Result as CoterieResult,
    integrations::{
        public_site::{sign_body, PublicSiteNotifier, PublicSiteOutcome, SIGNATURE_HEADER},
        Integration, IntegrationEvent,
    },
    repository::AnnouncementRepository,
    service::{
        announcement_admin_service::{
            AnnouncementAdminService, CreateAnnouncementInput, UpdateAnnouncementInput,
        },
        event_admin_service::{CreateEventInput, EventAdminService, UpdateEventInput},
        settings_service::{public_site_keys, SettingsService},
        ServiceContext,
    },
};
use sqlx::SqlitePool;
use tokio::sync::Mutex;
use tower::ServiceExt;
use uuid::Uuid;

mod common;
use common::{build_app_state, fresh_pool, make_member};

// ---------------------------------------------------------------------
// Recording integration — observes what the service dispatched.
// ---------------------------------------------------------------------

#[derive(Default)]
struct Recorder {
    seen: Mutex<Vec<IntegrationEvent>>,
}

impl Recorder {
    async fn take(&self) -> Vec<IntegrationEvent> {
        std::mem::take(&mut *self.seen.lock().await)
    }

    async fn names(&self) -> Vec<&'static str> {
        self.seen
            .lock()
            .await
            .iter()
            .map(|e| match e {
                IntegrationEvent::EventPublished(_) => "EventPublished",
                IntegrationEvent::EventUpdated(_) => "EventUpdated",
                IntegrationEvent::EventDeleted(_) => "EventDeleted",
                IntegrationEvent::AnnouncementPublished(_) => "AnnouncementPublished",
                IntegrationEvent::AnnouncementUpdated(_) => "AnnouncementUpdated",
                IntegrationEvent::AnnouncementDeleted(_) => "AnnouncementDeleted",
                _ => "other",
            })
            .collect()
    }
}

#[async_trait]
impl Integration for Recorder {
    fn name(&self) -> &str {
        "Recorder"
    }
    fn is_enabled(&self) -> bool {
        true
    }
    async fn health_check(&self) -> CoterieResult<()> {
        Ok(())
    }
    async fn handle_event(&self, event: &IntegrationEvent) -> CoterieResult<()> {
        self.seen.lock().await.push(event.clone());
        Ok(())
    }
}

/// The single `Event` carried by the one dispatched event variant,
/// panicking with the recorded variant names when that isn't the shape.
async fn only_event_payload(recorder: &Recorder, expect: &str) -> Event {
    let names = recorder.names().await;
    let seen = recorder.take().await;
    assert_eq!(names, vec![expect], "dispatched variants");
    match seen.into_iter().next().unwrap() {
        IntegrationEvent::EventUpdated(e) | IntegrationEvent::EventDeleted(e) => e,
        other => panic!("expected an event variant, got {other:?}"),
    }
}

// ---------------------------------------------------------------------
// Stub public site — observes what actually left the host.
// ---------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Received {
    body: String,
    signature: Option<String>,
}

type Inbox = Arc<Mutex<Vec<Received>>>;

/// How a stub receiver answers.
#[derive(Clone, Copy)]
enum Behaviour {
    Ok,
    ServerError,
    /// Accepts the request and never answers — the unresponsive-endpoint
    /// case the timeout bound exists for.
    Hang,
}

/// Spawn a receiver on an ephemeral port. Returns its endpoint URL and
/// the inbox every request lands in — an EMPTY inbox is what proves no
/// attempt was made, which is the assertion the unconfigured case needs.
async fn spawn_receiver(behaviour: Behaviour) -> (String, Inbox) {
    let inbox: Inbox = Arc::new(Mutex::new(Vec::new()));
    let state = (inbox.clone(), behaviour);

    async fn handle(
        State((inbox, behaviour)): State<(Inbox, Behaviour)>,
        headers: HeaderMap,
        body: String,
    ) -> StatusCode {
        inbox.lock().await.push(Received {
            body,
            signature: headers
                .get(SIGNATURE_HEADER)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string),
        });
        match behaviour {
            Behaviour::Ok => StatusCode::NO_CONTENT,
            Behaviour::ServerError => StatusCode::INTERNAL_SERVER_ERROR,
            Behaviour::Hang => {
                tokio::time::sleep(Duration::from_secs(120)).await;
                StatusCode::NO_CONTENT
            }
        }
    }

    let app = Router::new().route("/hook", post(handle)).with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/hook", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (url, inbox)
}

/// A URL nothing is listening on: bind, read the port, drop the socket.
async fn dead_endpoint() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{}/hook", addr)
}

// ---------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------

struct H {
    pool: SqlitePool,
    ctx: Arc<ServiceContext>,
    recorder: Arc<Recorder>,
    actor: Uuid,
}

impl H {
    fn events(&self) -> &EventAdminService {
        &self.ctx.event_admin_service
    }
    fn announcements(&self) -> &AnnouncementAdminService {
        &self.ctx.announcement_admin_service
    }
    fn settings(&self) -> &SettingsService {
        &self.ctx.settings_service
    }
    fn notifier(&self) -> &PublicSiteNotifier {
        &self.ctx.public_site_notifier
    }
    fn announcement_repo(&self) -> &dyn AnnouncementRepository {
        &*self.ctx.announcement_repo
    }

    /// Point the notifier at `url` with `secret`, through the same
    /// settings write path an admin uses (so the scheme check runs).
    async fn configure(&self, url: &str, secret: &str) {
        for (key, value) in [
            (public_site_keys::ENDPOINT_URL, url),
            (public_site_keys::SECRET, secret),
        ] {
            self.settings()
                .update_setting(
                    key,
                    UpdateSettingRequest {
                        value: value.to_string(),
                        reason: None,
                    },
                    self.actor,
                )
                .await
                .expect("configure public site");
        }
    }

    async fn create_event(&self, visibility: EventVisibility) -> Event {
        let event = self
            .events()
            .create(self.actor, event_input(visibility, None))
            .await
            .expect("create event");
        self.recorder.take().await;
        event
    }

    async fn update_event(
        &self,
        event: &Event,
        mutate: impl FnOnce(&mut UpdateEventInput),
    ) -> (Event, PublicSiteOutcome) {
        let mut input = update_input_from(event);
        mutate(&mut input);
        self.events()
            .update_one(self.actor, event.id, input)
            .await
            .expect("update event")
    }
}

async fn harness() -> H {
    let pool = fresh_pool().await;
    let actor = make_member(&pool).await;
    let state = build_app_state(pool.clone()).await;
    let ctx = state.service_context.clone();
    let recorder = Arc::new(Recorder::default());
    ctx.integration_manager.register(recorder.clone()).await;
    // Production registers the notifier on the same fan-out; tests that
    // exercise the bus half need it there too.
    ctx.integration_manager
        .register(ctx.public_site_notifier.clone())
        .await;
    H {
        pool,
        ctx,
        recorder,
        actor,
    }
}

fn soon() -> DateTime<Utc> {
    Utc::now() + ChronoDuration::days(3)
}

fn event_input(visibility: EventVisibility, recurrence: Option<Recurrence>) -> CreateEventInput {
    CreateEventInput {
        title: "Lockpicking 101".to_string(),
        description: "Bring your own padlock".to_string(),
        event_type: EventType::Workshop,
        event_type_id: None,
        visibility,
        start_time: soon(),
        end_time: None,
        timezone: "UTC".to_string(),
        location: Some("Back room".to_string()),
        max_attendees: Some(20),
        rsvp_required: false,
        member_price_cents: 0,
        guest_price_cents: 0,
        guest_registration_enabled: false,
        image_url: Some("https://example.com/pick.png".to_string()),
        recurrence,
        recurrence_until: None,
        series_pricing: Default::default(),
    }
}

fn update_input_from(e: &Event) -> UpdateEventInput {
    UpdateEventInput {
        title: e.title.clone(),
        description: e.description.clone(),
        event_type: e.event_type.clone(),
        event_type_id: e.event_type_id,
        visibility: e.visibility.clone(),
        start_time: e.start_time,
        end_time: e.end_time,
        location: e.location.clone(),
        max_attendees: e.max_attendees,
        rsvp_required: e.rsvp_required,
        member_price_cents: e.member_price_cents,
        guest_price_cents: e.guest_price_cents,
        guest_registration_enabled: e.guest_registration_enabled,
        image_url: e.image_url.clone(),
    }
}

fn announcement_input(is_public: bool, publish_now: bool) -> CreateAnnouncementInput {
    CreateAnnouncementInput {
        title: "Server room flooded".to_string(),
        content: "Details inside".to_string(),
        announcement_type: AnnouncementType::News,
        announcement_type_id: None,
        is_public,
        featured: false,
        image_url: None,
        publish_now,
        scheduled_publish_at: None,
        scheduled_publish_timezone: "UTC".to_string(),
    }
}

fn announcement_update(title: &str, is_public: bool) -> UpdateAnnouncementInput {
    UpdateAnnouncementInput {
        title: title.to_string(),
        content: "Details inside".to_string(),
        announcement_type: AnnouncementType::News,
        announcement_type_id: None,
        is_public,
        featured: false,
        image_url: None,
        scheduled_publish_at: None,
        scheduled_publish_timezone: "UTC".to_string(),
    }
}

// =====================================================================
// 6.1–6.9 — what gets dispatched
// =====================================================================

// 6.1 — update_one dispatches EventUpdated carrying the post-update state.
#[tokio::test]
async fn update_dispatches_event_updated_with_post_update_state() {
    let h = harness().await;
    let event = h.create_event(EventVisibility::Public).await;

    h.update_event(&event, |i| i.title = "Lockpicking 201".to_string())
        .await;

    let payload = only_event_payload(&h.recorder, "EventUpdated").await;
    assert_eq!(payload.id, event.id);
    assert_eq!(
        payload.title, "Lockpicking 201",
        "the dispatch carries the POST-update state, not the prior one",
    );
}

// 6.2 — delete dispatches EventDeleted; announcement update and delete
// dispatch their own variants.
#[tokio::test]
async fn delete_and_announcement_paths_dispatch_their_variants() {
    let h = harness().await;

    let event = h.create_event(EventVisibility::Public).await;
    h.events().delete_one(h.actor, event.id).await.unwrap();
    let payload = only_event_payload(&h.recorder, "EventDeleted").await;
    assert_eq!(payload.id, event.id);

    let announcement = h
        .announcements()
        .create(h.actor, announcement_input(true, true))
        .await
        .unwrap();
    h.recorder.take().await;

    h.announcements()
        .update(
            h.actor,
            announcement.id,
            announcement_update("Renamed", true),
        )
        .await
        .unwrap();
    assert_eq!(h.recorder.names().await, vec!["AnnouncementUpdated"]);
    h.recorder.take().await;

    h.announcements()
        .delete(h.actor, announcement.id)
        .await
        .unwrap();
    assert_eq!(h.recorder.names().await, vec!["AnnouncementDeleted"]);
}

// 6.3 — members-only → Public dispatches EventUpdated and NOT
// EventPublished. This is the gap that made the old design unable to see
// a late publication at all.
#[tokio::test]
async fn becoming_public_dispatches_update_not_published() {
    let h = harness().await;
    let event = h.create_event(EventVisibility::MembersOnly).await;

    h.update_event(&event, |i| i.visibility = EventVisibility::Public)
        .await;

    let names = h.recorder.names().await;
    assert_eq!(names, vec!["EventUpdated"]);
    assert!(
        !names.contains(&"EventPublished"),
        "EventPublished means 'created and not admin-only', never 'is now public'",
    );
}

// 6.4 — Public → AdminOnly dispatches an update carrying no title,
// description, location, or image.
#[tokio::test]
async fn withdrawal_to_admin_only_carries_no_content() {
    let h = harness().await;
    let event = h.create_event(EventVisibility::Public).await;

    h.update_event(&event, |i| i.visibility = EventVisibility::AdminOnly)
        .await;

    let payload = only_event_payload(&h.recorder, "EventUpdated").await;
    assert_eq!(payload.id, event.id, "identity is carried");
    assert_eq!(payload.visibility, EventVisibility::AdminOnly);
    assert!(payload.title.is_empty(), "title: {:?}", payload.title);
    assert!(payload.description.is_empty());
    assert_eq!(payload.location, None);
    assert_eq!(payload.image_url, None);
}

// 6.5 — a members-only item's dispatch carries no more than
// `/public/events` returns for it. Asserted against the projection's own
// output so the two cannot drift apart unnoticed.
#[tokio::test]
async fn members_only_dispatch_matches_the_public_projection() {
    let h = harness().await;
    let event = h.create_event(EventVisibility::MembersOnly).await;

    h.update_event(&event, |i| i.max_attendees = Some(5)).await;
    let payload = only_event_payload(&h.recorder, "EventUpdated").await;

    let app = coterie::api::create_app(build_app_state(h.pool.clone()).await);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/public/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let feed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let listed = feed
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == event.id.to_string())
        .expect("members-only event is listed (sanitized) on /public/events");

    assert_eq!(payload.title, listed["title"].as_str().unwrap());
    assert_eq!(payload.description, listed["description"].as_str().unwrap());
    assert!(listed["location"].is_null());
    assert!(listed["image_url"].is_null());
    assert_eq!(payload.location, None);
    assert_eq!(payload.image_url, None);
    assert!(
        listed["registration_url"].is_null() && payload.registration_url("").is_none(),
        "a members-only event advertises no public registration in either place",
    );
}

// 6.7 — an AdminOnly-to-AdminOnly edit dispatches nothing.
#[tokio::test]
async fn admin_only_edit_dispatches_nothing() {
    let h = harness().await;
    let event = h.create_event(EventVisibility::AdminOnly).await;

    h.update_event(&event, |i| {
        i.title = "Board session".to_string();
        i.start_time = soon() + ChronoDuration::days(1);
    })
    .await;

    assert!(
        h.recorder.names().await.is_empty(),
        "an item invisible before and after changes no public output",
    );
}

// 6.7 (second half) — an edit confined to fields the public projection
// omits is equally invisible.
#[tokio::test]
async fn edit_of_a_non_public_field_dispatches_nothing() {
    let h = harness().await;
    let event = h.create_event(EventVisibility::Public).await;

    // The member price is not part of `/public/events`.
    h.update_event(&event, |i| i.member_price_cents = 2500)
        .await;

    assert!(
        h.recorder.names().await.is_empty(),
        "member_price_cents is omitted from the public projection",
    );
}

// 6.8 — a repository failure dispatches nothing.
#[tokio::test]
async fn repository_failure_dispatches_nothing() {
    let h = harness().await;
    let event = h.create_event(EventVisibility::Public).await;

    h.pool.close().await;

    let err = h
        .events()
        .update_one(h.actor, event.id, update_input_from(&event))
        .await;
    assert!(err.is_err(), "a closed pool must fail the write");
    assert!(
        h.recorder.names().await.is_empty(),
        "no consumer is told about a change that did not persist",
    );
}

// 6.9 — cancelling an occurrence of a public series dispatches.
#[tokio::test]
async fn cancelling_an_occurrence_dispatches() {
    let h = harness().await;
    let anchor = next_weekday_anchor();
    let mut input = event_input(EventVisibility::Public, None);
    input.start_time = anchor;
    input.recurrence = Some(Recurrence::WeeklyByDay {
        interval: 1,
        weekdays: vec![WeekdayCode::Tue],
    });
    let first = h.events().create(h.actor, input).await.expect("series");
    let series_id = first.series_id.expect("series id");
    h.recorder.take().await;

    h.events()
        .cancel_event_occurrence(h.actor, series_id, 3, Some("holiday".into()))
        .await
        .expect("cancel occurrence");

    assert_eq!(
        h.recorder.names().await,
        vec!["EventDeleted"],
        "an occurrence leaving the public feed is a change to public output",
    );
}

/// Next Tuesday at 18:00, strictly after tomorrow — the weekly rule
/// requires its anchor to fall on the day it recurs.
fn next_weekday_anchor() -> DateTime<Utc> {
    use chrono::{Datelike, Weekday};
    let start = Utc::now() + ChronoDuration::days(1);
    let delta = (Weekday::Tue.num_days_from_monday() as i64
        - start.weekday().num_days_from_monday() as i64)
        .rem_euclid(7);
    (start.date_naive() + ChronoDuration::days(delta))
        .and_hms_opt(18, 0, 0)
        .unwrap()
        .and_utc()
}

// =====================================================================
// 6.6, 6.10–6.16 — what leaves the host
// =====================================================================

// 6.6 — no notification payload, for an item of any visibility, contains
// item content. This is the assertion that keeps the disclosure property
// structural rather than carefully maintained.
#[tokio::test]
async fn no_notification_payload_carries_item_content() {
    let h = harness().await;
    let (url, inbox) = spawn_receiver(Behaviour::Ok).await;
    h.configure(&url, "s3cret").await;

    for visibility in [
        EventVisibility::Public,
        EventVisibility::MembersOnly,
        EventVisibility::AdminOnly,
    ] {
        let event = h.create_event(visibility).await;
        h.update_event(&event, |i| i.title = "Padlock Night".to_string())
            .await;
        h.events().delete_one(h.actor, event.id).await.unwrap();
    }

    let announcement = h
        .announcements()
        .create(h.actor, announcement_input(true, true))
        .await
        .unwrap();
    h.announcements()
        .delete(h.actor, announcement.id)
        .await
        .unwrap();

    let received = inbox.lock().await.clone();
    assert!(!received.is_empty(), "notifications were sent");
    for r in &received {
        let body: serde_json::Value = serde_json::from_str(&r.body).expect("json body");
        let keys: Vec<&String> = body.as_object().unwrap().keys().collect();
        for content in [
            "Lockpicking",
            "Padlock Night",
            "Back room",
            "padlock",
            "pick.png",
            "Server room flooded",
            "Details inside",
        ] {
            assert!(
                !r.body.contains(content),
                "notification body leaked item content {content:?}: {}",
                r.body,
            );
        }
        assert!(body["kind"].is_string(), "kind is carried: {keys:?}");
        assert!(body["id"].is_string(), "id is carried: {keys:?}");
        assert!(body["action"].is_string(), "action is carried: {keys:?}");
    }
}

// 6.10 — with no endpoint configured, no HTTP attempt is made on any
// path. The receiver is live and reachable; its inbox staying EMPTY is
// what proves the absence of the attempt rather than merely the absence
// of an error.
#[tokio::test]
async fn unconfigured_makes_no_http_attempt_at_all() {
    let h = harness().await;
    let (_url, inbox) = spawn_receiver(Behaviour::Ok).await;

    let event = h.create_event(EventVisibility::Public).await;
    let (_, outcome) = h.update_event(&event, |i| i.title = "Renamed".into()).await;
    assert_eq!(outcome, PublicSiteOutcome::NotAttempted);

    let (_, withdrawal) = h
        .update_event(&event, |i| i.visibility = EventVisibility::AdminOnly)
        .await;
    assert_eq!(withdrawal, PublicSiteOutcome::NotAttempted);
    assert_eq!(withdrawal.admin_note(), None, "nothing to tell the admin");

    let deletion = h.events().delete_one(h.actor, event.id).await.unwrap();
    assert_eq!(deletion, PublicSiteOutcome::NotAttempted);

    let announcement = h
        .announcements()
        .create(h.actor, announcement_input(true, true))
        .await
        .unwrap();
    let (_, unpublished) = h
        .announcements()
        .unpublish(h.actor, announcement.id)
        .await
        .unwrap();
    assert_eq!(unpublished, PublicSiteOutcome::NotAttempted);

    assert!(
        inbox.lock().await.is_empty(),
        "an unconfigured deployment attempts nothing",
    );
    assert!(!h.notifier().is_configured().await);
}

// 6.11 — a withdrawal with a reachable endpoint reports success; with an
// unreachable one it reports failure, and the item is still withdrawn.
#[tokio::test]
async fn withdrawal_reports_success_or_failure_and_never_rolls_back() {
    let h = harness().await;
    let (url, _inbox) = spawn_receiver(Behaviour::Ok).await;
    h.configure(&url, "s3cret").await;

    let event = h.create_event(EventVisibility::Public).await;
    let (_, ok) = h
        .update_event(&event, |i| i.visibility = EventVisibility::AdminOnly)
        .await;
    assert_eq!(ok, PublicSiteOutcome::Sent);
    assert!(ok.admin_note().unwrap().contains("was updated"));

    // Now point at a port nothing is listening on.
    h.configure(&dead_endpoint().await, "s3cret").await;

    let announcement = h
        .announcements()
        .create(h.actor, announcement_input(true, true))
        .await
        .unwrap();
    let (saved, failed) = h
        .announcements()
        .unpublish(h.actor, announcement.id)
        .await
        .unwrap();

    assert!(failed.is_failed(), "got {failed:?}");
    let note = failed.admin_note().unwrap();
    assert!(note.contains("NOT updated"), "{note}");
    assert!(note.contains("Resend to public site"), "{note}");

    assert!(saved.published_at.is_none(), "the withdrawal stands");
    let stored = h
        .announcement_repo()
        .find_by_id(announcement.id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        stored.published_at.is_none(),
        "a failed notification never rolls the withdrawal back",
    );
}

// 6.12 — an endpoint that never responds does not hang the admin's
// request beyond the timeout bound.
#[tokio::test]
async fn unresponsive_endpoint_is_bounded_by_the_timeout() {
    let h = harness().await;
    let (url, _inbox) = spawn_receiver(Behaviour::Hang).await;
    h.configure(&url, "s3cret").await;

    let event = h.create_event(EventVisibility::Public).await;
    let started = Instant::now();
    let (_, outcome) = h
        .update_event(&event, |i| i.visibility = EventVisibility::AdminOnly)
        .await;
    let elapsed = started.elapsed();

    assert!(outcome.is_failed(), "got {outcome:?}");
    assert!(
        elapsed < Duration::from_secs(30),
        "the admin waited {elapsed:?} on an endpoint that never answers",
    );
}

// 6.13 — the signature verifies against the configured secret, and a
// body altered in transit fails verification.
#[tokio::test]
async fn signature_verifies_and_a_tampered_body_does_not() {
    let h = harness().await;
    let (url, inbox) = spawn_receiver(Behaviour::Ok).await;
    h.configure(&url, "shared-secret").await;

    let event = h.create_event(EventVisibility::Public).await;
    h.events().delete_one(h.actor, event.id).await.unwrap();

    // The last one is the deletion; the create rode the bus before it.
    let received = inbox.lock().await.clone();
    let r = received.last().expect("a notification arrived");
    let signature = r.signature.clone().expect("signature header present");
    assert!(r.body.contains("\"action\":\"deleted\""), "{}", r.body);

    assert_eq!(
        signature,
        sign_body("shared-secret", &r.body),
        "a receiver holding the shared secret can verify the body it got",
    );
    assert_ne!(
        signature,
        sign_body("shared-secret", &r.body.replace("deleted", "updated")),
        "a body altered in transit fails verification",
    );
    assert_ne!(
        signature,
        sign_body("guessed-secret", &r.body),
        "an arbitrary internet caller cannot forge one",
    );
}

// 6.14 — the secret never reaches the settings page or a failure message
// an operator sees. (The page's own masking is asserted in the unit test
// beside `setting_to_info`; this covers storage and the error path.)
#[tokio::test]
async fn the_secret_is_not_disclosed() {
    let h = harness().await;
    h.configure(&dead_endpoint().await, "top-secret-value")
        .await;

    let stored = h
        .settings()
        .get_setting(public_site_keys::SECRET)
        .await
        .unwrap();
    assert!(stored.is_sensitive, "the secret is a sensitive setting");
    assert!(
        !stored.value.contains("top-secret-value"),
        "the stored value is encrypted at rest",
    );

    for category in h.settings().get_all_settings().await.unwrap() {
        for setting in category.settings {
            assert!(
                !setting.value.contains("top-secret-value"),
                "{} handed back the plaintext secret",
                setting.key,
            );
        }
    }

    // The failure text is what gets rendered to the admin and logged.
    let event = h.create_event(EventVisibility::Public).await;
    let outcome = h.events().delete_one(h.actor, event.id).await.unwrap();
    let note = outcome.admin_note_deleted().unwrap();
    assert!(!note.contains("top-secret-value"), "{note}");
}

// 6.15 — resend sends for an item that is currently withdrawn. This is
// exactly how a missed withdrawal gets repaired.
#[tokio::test]
async fn resend_sends_for_a_withdrawn_item() {
    let h = harness().await;

    // Withdraw while the public site is unreachable, so the automatic
    // notification is lost.
    h.configure(&dead_endpoint().await, "s3cret").await;
    let event = h.create_event(EventVisibility::Public).await;
    let (withdrawn, lost) = h
        .update_event(&event, |i| i.visibility = EventVisibility::AdminOnly)
        .await;
    assert!(lost.is_failed());
    assert_eq!(withdrawn.visibility, EventVisibility::AdminOnly);

    // The site comes back; the admin hits resend on that one item.
    let (url, inbox) = spawn_receiver(Behaviour::Ok).await;
    h.configure(&url, "s3cret").await;

    let outcome = h
        .notifier()
        .resend(coterie::integrations::public_site::KIND_EVENT, event.id)
        .await;
    assert_eq!(outcome, PublicSiteOutcome::Sent);

    let received = inbox.lock().await.clone();
    assert_eq!(received.len(), 1, "one item, one notification");
    let body: serde_json::Value = serde_json::from_str(&received[0].body).unwrap();
    assert_eq!(body["id"], event.id.to_string());
    assert_eq!(body["kind"], "event");
}

// A receiver that answers non-2xx is a failure, not a success.
#[tokio::test]
async fn a_rejecting_receiver_is_reported_as_a_failure() {
    let h = harness().await;
    let (url, _inbox) = spawn_receiver(Behaviour::ServerError).await;
    h.configure(&url, "s3cret").await;

    let event = h.create_event(EventVisibility::Public).await;
    let outcome = h.events().delete_one(h.actor, event.id).await.unwrap();
    match outcome {
        PublicSiteOutcome::Failed(msg) => assert!(msg.contains("500"), "{msg}"),
        other => panic!("expected a failure, got {other:?}"),
    }
}

// A non-http(s) endpoint is rejected on write, so it can never reach the
// notifier at all.
#[tokio::test]
async fn a_non_http_endpoint_is_rejected_on_write() {
    let h = harness().await;
    let err = h
        .settings()
        .update_setting(
            public_site_keys::ENDPOINT_URL,
            UpdateSettingRequest {
                value: "javascript:alert(1)".to_string(),
                reason: None,
            },
            h.actor,
        )
        .await;
    assert!(err.is_err(), "a non-http(s) endpoint must not be stored");
    assert!(!h.notifier().is_configured().await);
}

// =====================================================================
// 6.16 — the resend control's visibility
// =====================================================================

// The control is present exactly when an endpoint is configured. Asserted
// against the rendered template, which is where the gate lives.
#[test]
fn resend_control_is_present_only_when_configured() {
    use askama::Template;
    use coterie::web::{
        portal::admin::events::{AdminEventDetail, AdminEventDetailTemplate},
        templates::BaseContext,
    };

    let render = |configured: bool| {
        AdminEventDetailTemplate {
            base: BaseContext::default(),
            event: AdminEventDetail {
                id: Uuid::nil().to_string(),
                title: "Lockpicking 101".to_string(),
                description: String::new(),
                event_type: "Workshop".to_string(),
                visibility: "Public".to_string(),
                start_time: "Jan 01, 2099 18:00".to_string(),
                start_time_input: "2099-01-01T18:00".to_string(),
                end_time: None,
                end_time_input: None,
                location: None,
                max_attendees: None,
                rsvp_required: false,
                image_url: None,
                attendee_count: 0,
                member_price_input: String::new(),
                member_price_display: "$0.00".to_string(),
                is_paid: false,
                guest_price_input: String::new(),
                guest_price_display: "Free".to_string(),
                guest_registration_enabled: false,
                registration_url: None,
                roster: Vec::new(),
                is_past: false,
                created_at: String::new(),
                updated_at: String::new(),
                is_series: false,
                occurrence_index: None,
                series_id: None,
            },
            event_types: Vec::new(),
            public_site_configured: configured,
        }
        .render()
        .expect("render event detail")
    };

    assert!(
        !render(false).contains("Resend to public site"),
        "no endpoint configured means no control at all",
    );
    let shown = render(true);
    assert!(shown.contains("Resend to public site"));
    assert!(shown.contains("/resend-public-site"));
}
