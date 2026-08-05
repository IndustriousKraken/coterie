//! The Coterie-hosted public registration page, `GET /events/:id/register`.
//!
//! Exists so an organizer can share ONE URL — in Discord, a newsletter, a
//! social post — without per-event work on any external site.
//!
//! It renders only fields that are already public for a `Public` event
//! and never the roster: who is attending is not public information. A
//! non-registerable event (members-only, admin-only, guest registration
//! off) and a nonexistent id both 404, because a 403 on a members-only id
//! would confirm the event exists to anyone enumerating ids.

use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use axum_extra::extract::CookieJar;
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    api::middleware::auth::optional_member,
    auth::{AuthService, CsrfService},
    domain::{AttendanceStatus, Attendee},
    repository::{EventRepository, MemberRepository},
    service::settings_service::{bot_challenge_keys, SettingsService},
    web::templates::{BaseContext, HtmlTemplate},
};

#[derive(Debug, Deserialize)]
pub struct RegisterPageQuery {
    /// Set by the redirect after a successful form post, so a guest
    /// returning from Stripe (or from a free registration) sees that it
    /// worked. Purely cosmetic — the seat's real state lives in the DB.
    pub registered: Option<String>,
}

/// What a shareable registration page knows about a signed-in member.
///
/// `None` on a template means the visitor is anonymous — the guest
/// rendering, unchanged. Shared by the event page and the class page:
/// they are the same page at two scopes.
pub struct SignedInMember {
    /// The member price for this event or class, rendered `"Free"` for a
    /// zero price exactly as the guest price is.
    pub price_display: String,
    /// True when they already hold a seat (or a class pass), so the page
    /// says so instead of offering an action that looks like a second
    /// charge.
    pub already_registered: bool,
}

/// True when this attendance state means "don't offer to sell them the
/// seat again" — a confirmed seat OR one held in flight at Stripe.
/// Mirrors the portal's own enrolled/registered test.
pub(crate) fn holds_a_seat(status: Option<AttendanceStatus>) -> bool {
    matches!(
        status,
        Some(AttendanceStatus::Registered) | Some(AttendanceStatus::PendingPayment)
    )
}

#[derive(Template)]
#[template(path = "events/register.html")]
pub struct EventRegisterTemplate {
    pub base: BaseContext,
    pub event_id: String,
    pub title: String,
    /// The description as sanitized safe-subset HTML from the shared
    /// Markdown pipeline. The raw Markdown is deliberately NOT on the
    /// template: `|safe` may only ever meet rendered output.
    pub description_html: String,
    /// Wall-clock in the event's own zone, labeled with the zone
    /// abbreviation — the guest may be anywhere.
    pub when: String,
    pub location: Option<String>,
    /// `"Free"` rather than `"$0.00"`: a zero price is free, and
    /// rendering it as money reads like a bug.
    pub guest_price_display: String,
    /// Member price + login link, shown only when it differs from the
    /// guest price, so a member about to overpay is told before paying
    /// rather than silently re-priced off an unverified email.
    pub member_price_display: Option<String>,
    /// Rendered "N seats remaining", or `None` for an uncapped event.
    /// Pluralized here rather than in the template so the template stays
    /// a template.
    pub seats_remaining: Option<String>,
    pub is_full: bool,
    /// Turnstile site key when a provider is configured; the widget (and
    /// the token it produces) is only rendered when there is one.
    pub captcha_site_key: Option<String>,
    pub just_registered: bool,
    /// The visitor's member session, when they have one. `None` renders
    /// today's guest page; `Some` renders the member-priced authenticated
    /// action instead of the guest form.
    pub signed_in: Option<SignedInMember>,
}

// Granular state per the routing-architecture spec: the page reads five
// unrelated dependencies (event, settings, session, member, CSRF) and
// bundling them into one extractor would hide which it actually uses.
#[allow(clippy::too_many_arguments)]
pub async fn event_register_page(
    State(event_repo): State<Arc<dyn EventRepository>>,
    State(settings_service): State<Arc<SettingsService>>,
    State(auth_service): State<Arc<AuthService>>,
    State(member_repo): State<Arc<dyn MemberRepository>>,
    State(csrf_service): State<Arc<CsrfService>>,
    jar: CookieJar,
    Path(event_id): Path<Uuid>,
    Query(query): Query<RegisterPageQuery>,
) -> Response {
    // One rule, one home: `publicly_registerable`. Anything else is
    // indistinguishable from "no such event".
    let event = match event_repo.find_by_id(event_id).await {
        Ok(Some(e)) if e.publicly_registerable() => e,
        Ok(_) => return not_found(),
        Err(e) => {
            tracing::error!("Registration page lookup failed for {}: {}", event_id, e);
            return not_found();
        }
    };

    // Capacity is the held-seat count — the same predicate the claim
    // enforces — so the page and the endpoint agree about "full".
    let held = event_repo.count_held_seats(event.id).await.unwrap_or(0);
    let seats_remaining = event
        .max_attendees
        .map(|max| (i64::from(max) - held).max(0));
    let is_full = seats_remaining == Some(0);

    // Render the challenge widget only when a provider is actually
    // configured — the endpoint fails closed on a missing token, so a
    // page with no widget against an active provider would be a form
    // nobody can submit. Only the (public) site key is read here; the
    // secret never touches this path.
    let provider = settings_service
        .get_value(bot_challenge_keys::PROVIDER)
        .await
        .unwrap_or_else(|_| "disabled".to_string());
    let captcha_site_key = if provider == "disabled" {
        None
    } else {
        settings_service
            .get_value(bot_challenge_keys::SITE_KEY)
            .await
            .ok()
            .filter(|k| !k.is_empty())
    };

    // Resolve the session AFTER the registerability 404, so a session
    // never widens what is visible. Fails open: `optional_member`
    // returns `None` for every failure mode, and `None` is exactly
    // today's anonymous page.
    let (base, signed_in) = match optional_member(&auth_service, &*member_repo, &jar).await {
        Some((user, session)) => {
            let already_registered = holds_a_seat(
                event_repo
                    .attendance_status(event.id, &Attendee::Member(user.member.id))
                    .await
                    .ok()
                    .flatten(),
            );
            (
                // `for_member`, not `for_anon`: the authenticated action
                // is CSRF-protected and the token rides in the layout.
                BaseContext::for_member(&csrf_service, &user, &session).await,
                Some(SignedInMember {
                    price_display: if event.is_paid_for_members() {
                        format!("${:.2}", event.member_price_cents as f64 / 100.0)
                    } else {
                        "Free".to_string()
                    },
                    already_registered,
                }),
            )
        }
        None => (BaseContext::for_anon(), None),
    };

    let template = EventRegisterTemplate {
        base,
        event_id: event.id.to_string(),
        title: event.title.clone(),
        description_html: crate::util::markdown::render_markdown(&event.description),
        when: format!(
            "{} {}",
            event.start_time.format("%A, %B %-d, %Y at %-I:%M %p"),
            event.zone_abbr(),
        ),
        location: event.location.clone(),
        guest_price_display: if event.is_paid_for_guests() {
            format!("${:.2}", event.guest_price_cents as f64 / 100.0)
        } else {
            "Free".to_string()
        },
        member_price_display: (event.member_price_cents != event.guest_price_cents).then(|| {
            if event.is_paid_for_members() {
                format!("${:.2}", event.member_price_cents as f64 / 100.0)
            } else {
                "free".to_string()
            }
        }),
        seats_remaining: seats_remaining
            .map(|n| format!("{} seat{} remaining", n, if n == 1 { "" } else { "s" })),
        is_full,
        captcha_site_key,
        just_registered: query.registered.is_some(),
        signed_in,
    };
    HtmlTemplate(template).into_response()
}

/// The one response a non-registerable event, a bad id, and a lookup
/// error all share.
fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "Not Found").into_response()
}
