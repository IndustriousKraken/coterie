//! OpenAPI specification for the public API surface.
//!
//! Only endpoints intended for the public website integration are
//! documented here (signup, public event/announcement reads, donations,
//! RSS/iCal feeds, plus root/health metadata). Authenticated portal
//! routes are deliberately excluded.

use utoipa::OpenApi;

use crate::api::handlers;
use crate::domain;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Coterie Public API",
        description = "Public API surface consumed by the marketing site (signup, event \
            and announcement reads, donations, calendar/RSS feeds). Authenticated portal \
            and admin endpoints are intentionally not documented here.",
        version = env!("CARGO_PKG_VERSION"),
    ),
    paths(
        handlers::root::root,
        handlers::root::health_check,
        handlers::root::api_info,
        handlers::public::signup,
        handlers::public::list_events,
        handlers::public::private_event_count,
        handlers::public::list_announcements,
        handlers::public::rss_feed,
        handlers::public::calendar_feed,
        handlers::public::donate,
        handlers::announcements::private_count,
    ),
    components(schemas(
        // Root metadata
        handlers::root::ApiInfo,
        handlers::root::HealthStatus,
        // Public DTOs
        handlers::public::SignupRequest,
        handlers::public::SignupResponse,
        handlers::public::PrivateEventCount,
        handlers::public::PublicDonateRequest,
        handlers::public::PublicDonateResponse,
        handlers::announcements::PrivateAnnouncementCount,
        // Domain types referenced from responses
        domain::Event,
        domain::EventType,
        domain::EventVisibility,
        domain::Announcement,
        domain::AnnouncementType,
        domain::MemberStatus,
        domain::MembershipType,
    )),
    tags(
        (name = "public", description = "Public API for website integration"),
        (name = "meta", description = "Service metadata and health checks"),
    ),
)]
pub struct ApiDoc;
