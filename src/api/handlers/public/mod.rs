//! Public, unauthenticated API surface, split by subject: [`signup`]
//! (the join funnel and its Stripe Checkout), [`feeds`] (RSS/iCal
//! serialization), [`donate`] (public donations), and [`register`]
//! (guest event registration and class enrollment).
//!
//! What stays here are the read projections the marketing site consumes
//! and the public DTOs they share.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{
    config::Settings,
    domain::{is_upcoming, Announcement, AnnouncementType, Event, EventType, EventVisibility},
    error::Result,
    repository::{AnnouncementRepository, EventRepository},
    service::membership_type_service::MembershipTypeService,
};

mod donate;
mod feeds;
mod register;
mod signup;

// Re-exported flat so `handlers::public::<item>` keeps resolving for
// `src/api/mod.rs`, `src/api/docs.rs` (including the `__path_*` types
// `#[utoipa::path]` generates), and `src/web/templates/verify.rs`.
pub use donate::*;
pub use feeds::*;
pub use register::*;
pub use signup::*;

/// Public projection of a membership type for the join form. Deliberately
/// excludes internal fields (`id`, `is_active`, timestamps) — the slug is
/// the public identifier and is what `POST /public/signup` accepts.
#[derive(Debug, Serialize, ToSchema)]
pub struct PublicMembershipType {
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub fee_cents: i32,
    pub currency: String,
    /// One of `monthly`, `yearly`, `lifetime`.
    pub billing_period: String,
}

/// Public projection of an `Event` for `GET /public/events`. Exposes
/// only the fields the marketing site consumes and deliberately omits
/// internal identifiers that must never reach anonymous callers —
/// `created_by` (the organizer's member id), `created_at`, `updated_at`,
/// `event_type_id`, `series_id`, and `occurrence_index`. Members-only
/// sanitization (nulling title/description/location/image_url) is applied
/// to the source `Event` before projection, so it carries through.
#[derive(Debug, Serialize, ToSchema)]
pub struct PublicEvent {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub event_type: EventType,
    pub visibility: EventVisibility,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub timezone: String,
    pub location: Option<String>,
    pub image_url: Option<String>,
    pub max_attendees: Option<i32>,
    pub rsvp_required: bool,
    /// Absolute URL of the Coterie-hosted public registration page, or
    /// `null` when the event is not publicly registerable. Present
    /// exactly when `guest_price_cents` is, and it is the ONLY thing a
    /// consumer should test to decide whether to offer registration —
    /// re-deriving that from price/visibility/`rsvp_required` would be a
    /// second implementation of a server-side authorization rule.
    ///
    /// Most events carry `null` here: the ordinary recurring talk anyone
    /// may walk into has no guest registration enabled. Presence is the
    /// unusual condition worth surfacing.
    pub registration_url: Option<String>,
    /// What a non-member pays, in cents, or `null` when the event is not
    /// publicly registerable. `0` means free — and a zero price never
    /// suppresses `registration_url`.
    pub guest_price_cents: Option<i64>,
}

impl PublicEvent {
    /// Project an event for the public feed. `base_url` resolves the
    /// registration URL server-side; the two guest fields are populated
    /// together from `Event::registration_url` or not at all, so they
    /// can't disagree about registerability.
    fn from_event(e: Event, base_url: &str) -> Self {
        let registration_url = e.registration_url(base_url);
        let guest_price_cents = registration_url.as_ref().map(|_| e.guest_price_cents);
        PublicEvent {
            id: e.id,
            title: e.title,
            description: e.description,
            event_type: e.event_type,
            visibility: e.visibility,
            start_time: e.start_time,
            end_time: e.end_time,
            timezone: e.timezone,
            location: e.location,
            image_url: e.image_url,
            max_attendees: e.max_attendees,
            rsvp_required: e.rsvp_required,
            registration_url,
            guest_price_cents,
        }
    }
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct PublicEventsQuery {
    /// Maximum number of events to return (default 50).
    pub limit: Option<i64>,
    /// Response format: omit or `"json"` for JSON; `"ical"` for an
    /// iCal/.ics calendar feed.
    pub format: Option<String>,
    /// Optional inclusive start of a date range (RFC 3339 instant). When
    /// both `from` and `to` are supplied, valid, `to > from`, and the span
    /// is within the maximum window, the JSON feed returns events whose
    /// derived UTC instant is in `[from, to)` — **including past events** —
    /// instead of the default upcoming-only list. Ignored for `format=ical`
    /// and silently ignored (falls back to upcoming-only) if malformed.
    pub from: Option<String>,
    /// Optional exclusive end of the date range (RFC 3339 instant). See `from`.
    pub to: Option<String>,
}

/// Maximum span of a `from`/`to` range on `GET /public/events`. Bounds the
/// scan an anonymous caller can request; a wider (or malformed) range falls
/// back to the default upcoming-only list. ~400 days covers a full calendar
/// month view (with adjacent-month spill) plus slack.
const MAX_RANGE_SPAN_DAYS: i64 = 400;

/// Parse the opt-in `from`/`to` range. Returns `Some((from, to))` only when
/// BOTH parse as RFC 3339 instants, `to > from`, and the window is no wider
/// than `MAX_RANGE_SPAN_DAYS`; otherwise `None`, so the caller falls back to
/// the default upcoming-only filter (a bad range must never error).
fn parse_range(
    from: &Option<String>,
    to: &Option<String>,
) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let from = DateTime::parse_from_rfc3339(from.as_deref()?)
        .ok()?
        .with_timezone(&Utc);
    let to = DateTime::parse_from_rfc3339(to.as_deref()?)
        .ok()?
        .with_timezone(&Utc);
    (to > from && to - from <= Duration::days(MAX_RANGE_SPAN_DAYS)).then_some((from, to))
}

#[utoipa::path(
    get,
    path = "/public/events",
    tag = "public",
    params(PublicEventsQuery),
    responses(
        (status = 200, description = "Upcoming public + sanitized members-only events", body = [PublicEvent],
            content_type = "application/json"),
        (status = 200, description = "iCal feed (when format=ical)", content_type = "text/calendar"),
    ),
)]
pub async fn list_events(
    State(event_repo): State<Arc<dyn EventRepository>>,
    State(settings): State<Arc<Settings>>,
    Query(params): Query<PublicEventsQuery>,
) -> Result<Response> {
    // Get public events (full details)
    let public_events = event_repo.list_public().await?;

    // Get members-only events (will be sanitized)
    let private_events = event_repo.list_members_only().await?;

    // Combine, then replace each event's stored wall-clock with its
    // derived UTC instant so the "upcoming" filter, the sort, and the
    // JSON/iCal output all compare and emit true instants (not the
    // naive wall-clock, which would be off by the org's offset).
    let now = Utc::now();
    let mut events: Vec<Event> = public_events
        .into_iter()
        .chain(private_events.into_iter().map(|mut e| {
            // Sanitize private events. `guest_registration_enabled` is
            // cleared alongside the other fields so a members-only event
            // can never advertise a public registration URL — the
            // projection derives that from the event, and this is where
            // members-only events stop being registerable.
            e.title = "Members-Only Event".to_string();
            e.description =
                "This event is for members only. Log in to the portal to see details.".to_string();
            e.location = None;
            e.image_url = None;
            e.guest_registration_enabled = false;
            e
        }))
        .collect();
    // `start_time`/`end_time` now hold the derived UTC instant, so the
    // filters below compare true instants (not the naive wall-clock).
    derive_utc_instants(&mut events);

    // iCal is ALWAYS upcoming-only (the home page + calendar subscriptions
    // depend on it); the range opt-in applies to the JSON feed only. A range
    // is honored only when both `from`/`to` parse, `to > from`, and the span
    // is bounded — otherwise we fall back to the upcoming filter unchanged.
    let is_ical = params.format.as_deref() == Some("ical");
    let range = if is_ical {
        None
    } else {
        parse_range(&params.from, &params.to)
    };
    match range {
        Some((from, to)) => events.retain(|e| e.start_time >= from && e.start_time < to),
        // `is_upcoming` (the free function) and NOT `Event::is_upcoming`:
        // `derive_utc_instants` above already replaced both fields with
        // their derived instants, and the method would re-derive — adding
        // the org's offset a second time and putting the marketing feed
        // and every calendar subscription hours out.
        None => events.retain(|e| is_upcoming(e.start_time, e.end_time, now)),
    }

    // Sort by start time
    events.sort_by(|a, b| a.start_time.cmp(&b.start_time));

    // Apply limit
    events.truncate(params.limit.unwrap_or(50) as usize);

    if is_ical {
        let ical = generate_ical_feed(&events);
        Ok((
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/calendar; charset=utf-8")],
            ical,
        )
            .into_response())
    } else {
        // Project to PublicEvent so internal-only fields (created_by,
        // timestamps, event_type_id, series_id, occurrence_index) never
        // reach anonymous callers. Sanitization already ran above.
        let public: Vec<PublicEvent> = events
            .into_iter()
            .map(|e| PublicEvent::from_event(e, &settings.server.base_url))
            .collect();
        Ok(Json(public).into_response())
    }
}

/// Public projection of an `Announcement` for `GET /public/announcements`.
/// Exposes only the fields the marketing site consumes and deliberately
/// omits internal identifiers/implementation detail that must never reach
/// anonymous callers — `created_by` (the author's member id), `created_at`,
/// `updated_at`, `announcement_type_id`, `is_public`, and the scheduling
/// fields (`scheduled_publish_at`, `scheduled_publish_timezone`). Mirrors
/// `PublicEvent`.
///
/// Alongside the raw Markdown `content` it carries a server-rendered
/// sanitized `content_html` (Markdown → safe-subset HTML) so a consumer can
/// render formatted content without running its own Markdown parser or
/// making a sanitization decision.
#[derive(Debug, Serialize, ToSchema)]
pub struct PublicAnnouncement {
    pub id: Uuid,
    pub title: String,
    /// Raw Markdown source.
    pub content: String,
    /// Server-rendered sanitized safe-subset HTML of `content`.
    pub content_html: String,
    pub announcement_type: AnnouncementType,
    pub featured: bool,
    pub image_url: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
}

impl From<Announcement> for PublicAnnouncement {
    fn from(a: Announcement) -> Self {
        PublicAnnouncement {
            content_html: crate::util::markdown::render_announcement_markdown(&a.content),
            id: a.id,
            title: a.title,
            content: a.content,
            announcement_type: a.announcement_type,
            featured: a.featured,
            image_url: a.image_url,
            published_at: a.published_at,
        }
    }
}

#[utoipa::path(
    get,
    path = "/public/announcements",
    tag = "public",
    responses(
        (status = 200, description = "Published public announcements, each with a \
            server-rendered sanitized `content_html` alongside the raw Markdown `content`",
            body = [PublicAnnouncement]),
    ),
)]
pub async fn list_announcements(
    State(announcement_repo): State<Arc<dyn AnnouncementRepository>>,
) -> Result<Json<Vec<PublicAnnouncement>>> {
    // Get public announcements only
    let announcements = announcement_repo.list_public().await?;

    // Filter to published announcements only, then project to
    // PublicAnnouncement so internal-only fields (created_by, timestamps,
    // announcement_type_id, is_public, scheduled_publish_*) never reach
    // anonymous callers. The projection also attaches the sanitized
    // server-rendered HTML from the shared Markdown pipeline.
    let published: Vec<PublicAnnouncement> = announcements
        .into_iter()
        .filter(|a| a.published_at.is_some())
        .map(PublicAnnouncement::from)
        .collect();

    Ok(Json(published))
}

#[utoipa::path(
    get,
    path = "/public/membership-types",
    tag = "public",
    responses(
        (status = 200, description = "Active membership types, ordered by sort_order, \
            for the public join form", body = [PublicMembershipType]),
    ),
)]
pub async fn list_membership_types(
    State(membership_type_service): State<Arc<MembershipTypeService>>,
) -> Result<Json<Vec<PublicMembershipType>>> {
    // Active types only; the repo already orders by sort_order, name.
    let types = membership_type_service.list(false).await?;
    Ok(Json(
        types
            .into_iter()
            .map(|t| PublicMembershipType {
                slug: t.slug,
                name: t.name,
                description: t.description,
                fee_cents: t.fee_cents,
                // Payments are USD throughout (see stripe_client); emit it
                // explicitly so the form can render without assuming.
                currency: "USD".to_string(),
                billing_period: t.billing_period,
            })
            .collect(),
    ))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PrivateEventCount {
    pub count: i64,
}

#[utoipa::path(
    get,
    path = "/public/events/private-count",
    tag = "public",
    responses(
        (status = 200, description = "Count of upcoming members-only events", body = PrivateEventCount),
    ),
)]
pub async fn private_event_count(
    State(event_repo): State<Arc<dyn EventRepository>>,
) -> Result<Json<PrivateEventCount>> {
    let count = event_repo.count_members_only_upcoming().await?;
    Ok(Json(PrivateEventCount { count }))
}
/// Replace each event's stored wall-clock `start_time`/`end_time` with
/// its derived UTC instant (from the event's IANA zone), in place. Once
/// applied, downstream serialization — the `…Z` JSON timestamps and the
/// iCal `DTSTART`/`DTEND` — emits correct instants without any further
/// per-call conversion. Idempotent only if called once per read path;
/// callers run it exactly once before filtering/sorting/serializing.
fn derive_utc_instants(events: &mut [Event]) {
    for e in events.iter_mut() {
        let start = e.start_utc();
        let end = e.end_utc();
        e.start_time = start;
        e.end_time = end;
    }
}

#[cfg(test)]
mod upcoming_derivation_tests {
    //! 3.7 — the double-conversion guard. `derive_utc_instants` replaces
    //! `start_time`/`end_time` with derived instants IN PLACE, so this
    //! surface must call the free `is_upcoming` and never
    //! `Event::is_upcoming`, which would derive a second time.

    use super::*;
    use chrono::TimeZone;

    // 19:00–21:00 on Jan 23 in New York (EST, UTC-5) → 00:00Z–02:00Z on
    // the 24th. A UTC fixture could not detect the bug: the offset it
    // would add is zero.
    fn ny_evening() -> Event {
        let wall = |h: u32| {
            DateTime::from_naive_utc_and_offset(
                chrono::NaiveDate::from_ymd_opt(2026, 1, 23)
                    .unwrap()
                    .and_hms_opt(h, 0, 0)
                    .unwrap(),
                Utc,
            )
        };
        Event {
            id: Uuid::new_v4(),
            title: "HTB Night".into(),
            description: String::new(),
            event_type: EventType::Meeting,
            event_type_id: None,
            visibility: EventVisibility::Public,
            start_time: wall(19),
            end_time: Some(wall(21)),
            timezone: "America/New_York".into(),
            location: None,
            max_attendees: None,
            rsvp_required: false,
            member_price_cents: 0,
            guest_price_cents: 0,
            guest_registration_enabled: false,
            image_url: None,
            created_by: Uuid::new_v4(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            series_id: None,
            occurrence_index: None,
        }
    }

    #[test]
    fn derived_instants_answer_the_same_as_the_undebased_event() {
        // 04:00Z on the 24th: two hours past the true end (02:00Z), but
        // still inside the window a second conversion would invent.
        let now = Utc.with_ymd_and_hms(2026, 1, 24, 4, 0, 0).unwrap();
        let mut events = vec![ny_evening()];

        // Before derivation, the convenience method is the right call.
        let before = events[0].is_upcoming(now);
        assert!(!before, "the event ended at 02:00Z; 04:00Z is past it");

        derive_utc_instants(&mut events);
        let after = is_upcoming(events[0].start_time, events[0].end_time, now);
        assert_eq!(
            before, after,
            "the derived surface must agree with the undebased one"
        );

        // And this is what taking the convenient path would have cost:
        // the wrapper re-derives 02:00Z as a New York wall-clock (07:00Z)
        // and keeps the event listed five hours past its end.
        assert!(
            events[0].is_upcoming(now),
            "guard is only meaningful while double conversion actually shifts \
             the answer — if this ever fails the fixture stopped testing anything"
        );
    }

    // The in-progress case on the surface that matters most: 01:00Z is
    // between the derived start (00:00Z) and end (02:00Z).
    #[test]
    fn an_event_in_progress_survives_derivation() {
        let now = Utc.with_ymd_and_hms(2026, 1, 24, 1, 0, 0).unwrap();
        let mut events = vec![ny_evening()];
        assert!(events[0].is_upcoming(now));
        derive_utc_instants(&mut events);
        assert!(is_upcoming(events[0].start_time, events[0].end_time, now));
    }
}

#[cfg(test)]
mod announcement_markdown_tests {
    //! Public-feed rendering: `/public/announcements` carries a sanitized
    //! `content_html`, and the RSS item description carries the same
    //! sanitized rendered HTML. Both flow through the shared pipeline
    //! (`crate::util::markdown::render_announcement_markdown`).

    use super::feeds::generate_rss_feed;
    use super::*;
    use axum::{body::Body, http::Request, routing::get, Router};
    use chrono::Utc;
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::{
        domain::{Announcement, AnnouncementType, CreateMemberRequest, Member},
        repository::{MemberRepository, SqliteAnnouncementRepository, SqliteMemberRepository},
    };

    // A Markdown body exercising every relevant case: formatting that must
    // render, plus disallowed constructs that must be stripped.
    const RICH_BODY: &str = "**bold** *italic* ~~struck~~\n\n\
        A [safe link](https://example.com) and a [bad link](javascript:alert(1)).\n\n\
        <script>alert(2)</script>\n\n\
        ![img](https://example.com/x.png)";

    fn assert_sanitized(html: &str) {
        assert!(
            html.contains("<strong>bold</strong>"),
            "bold rendered: {html}"
        );
        assert!(html.contains("<em>italic</em>"), "italic rendered: {html}");
        assert!(
            html.contains("<del>struck</del>"),
            "strike rendered: {html}"
        );
        assert!(
            html.contains("href=\"https://example.com\""),
            "safe https link preserved: {html}"
        );
        assert!(!html.contains("<script"), "no live script element: {html}");
        assert!(
            !html.contains("javascript:"),
            "no javascript: scheme: {html}"
        );
        assert!(!html.contains("<img"), "no img element: {html}");
    }

    async fn migrated_pool() -> sqlx::SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn public_announcements_entry_carries_sanitized_content_html() {
        let pool = migrated_pool().await;

        let member_repo: Arc<dyn MemberRepository> =
            Arc::new(SqliteMemberRepository::new(pool.clone()));
        let member: Member = member_repo
            .create(CreateMemberRequest {
                email: "admin@example.com".to_string(),
                username: "admin".to_string(),
                full_name: "Admin".to_string(),
                password: "p4ssword_long_enough".to_string(),
                membership_type_id: None,
                ..Default::default()
            })
            .await
            .unwrap();

        let announcement_repo: Arc<dyn AnnouncementRepository> =
            Arc::new(SqliteAnnouncementRepository::new(pool.clone()));
        let now = Utc::now();
        announcement_repo
            .create(Announcement {
                id: Uuid::new_v4(),
                title: "Rich".to_string(),
                content: RICH_BODY.to_string(),
                announcement_type: AnnouncementType::General,
                announcement_type_id: None,
                is_public: true,
                featured: false,
                image_url: None,
                published_at: Some(now),
                scheduled_publish_at: None,
                scheduled_publish_timezone: "UTC".to_string(),
                created_by: member.id,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        let app = Router::new()
            .route("/public/announcements", get(list_announcements))
            .with_state(announcement_repo);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/public/announcements")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let entry = &json.as_array().expect("array")[0];

        let content_html = entry
            .get("content_html")
            .and_then(|v| v.as_str())
            .expect("content_html field present");
        assert_sanitized(content_html);

        // Raw Markdown source is kept alongside the rendered HTML.
        let raw = entry.get("content").and_then(|v| v.as_str()).unwrap();
        assert_eq!(raw, RICH_BODY, "raw content preserved");
    }

    #[tokio::test]
    async fn public_announcements_omit_internal_fields() {
        let pool = migrated_pool().await;

        let member_repo: Arc<dyn MemberRepository> =
            Arc::new(SqliteMemberRepository::new(pool.clone()));
        let member: Member = member_repo
            .create(CreateMemberRequest {
                email: "admin@example.com".to_string(),
                username: "admin".to_string(),
                full_name: "Admin".to_string(),
                password: "p4ssword_long_enough".to_string(),
                membership_type_id: None,
                ..Default::default()
            })
            .await
            .unwrap();

        let announcement_repo: Arc<dyn AnnouncementRepository> =
            Arc::new(SqliteAnnouncementRepository::new(pool.clone()));
        let now = Utc::now();
        announcement_repo
            .create(Announcement {
                id: Uuid::new_v4(),
                title: "Public".to_string(),
                content: "Body".to_string(),
                announcement_type: AnnouncementType::General,
                announcement_type_id: None,
                is_public: true,
                featured: true,
                image_url: Some("https://example.com/x.png".to_string()),
                published_at: Some(now),
                scheduled_publish_at: Some(now),
                scheduled_publish_timezone: "America/New_York".to_string(),
                created_by: member.id,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        let app = Router::new()
            .route("/public/announcements", get(list_announcements))
            .with_state(announcement_repo);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/public/announcements")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = json.as_array().expect("array");
        assert_eq!(arr.len(), 1, "one published public announcement");
        let entry = arr[0].as_object().expect("object");

        // Internal fields must NOT reach the anonymous marketing surface.
        for internal in [
            "created_by",
            "created_at",
            "updated_at",
            "announcement_type_id",
            "is_public",
            "scheduled_publish_at",
            "scheduled_publish_timezone",
        ] {
            assert!(
                !entry.contains_key(internal),
                "internal field `{internal}` leaked: {entry:?}",
            );
        }

        // The projected public field set is present and unchanged.
        for public in [
            "id",
            "title",
            "content",
            "content_html",
            "announcement_type",
            "featured",
            "image_url",
            "published_at",
        ] {
            assert!(
                entry.contains_key(public),
                "public field `{public}` missing: {entry:?}",
            );
        }
    }

    #[test]
    fn rss_description_carries_sanitized_rendered_html() {
        let now = Utc::now();
        let announcement = Announcement {
            id: Uuid::new_v4(),
            title: "Rich".to_string(),
            content: RICH_BODY.to_string(),
            announcement_type: AnnouncementType::General,
            announcement_type_id: None,
            is_public: true,
            featured: false,
            image_url: None,
            published_at: Some(now),
            scheduled_publish_at: None,
            scheduled_publish_timezone: "UTC".to_string(),
            created_by: Uuid::new_v4(),
            created_at: now,
            updated_at: now,
        };

        let rss = generate_rss_feed(&[announcement]);
        // The description block carries the sanitized rendered HTML.
        assert!(
            rss.contains("<description><![CDATA[") && rss.contains("<strong>bold</strong>"),
            "rss description carries rendered HTML: {rss}"
        );
        assert!(
            !rss.contains("<script"),
            "no live script element in rss: {rss}"
        );
        assert!(
            !rss.contains("javascript:"),
            "no javascript: scheme in rss: {rss}"
        );
    }
}
