//! The Coterie-hosted public class page, `GET /classes/:id/register`.
//!
//! a42's registration page at series scope, and it follows the same rules
//! for the same reasons: it renders only what is already public about the
//! class, never the roster, and a non-enrollable series (occurrences not
//! `Public`, guest enrollment off) 404s exactly like a nonexistent id —
//! a 403 on a members-only class would confirm it exists to anyone
//! enumerating ids.

use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use axum_extra::extract::CookieJar;
use uuid::Uuid;

use crate::{
    api::middleware::auth::optional_member,
    auth::{AuthService, CsrfService},
    domain::Attendee,
    repository::{
        EventRepository, EventSeriesRepository, MemberRepository, SeriesEnrollmentRepository,
    },
    service::settings_service::{bot_challenge_keys, SettingsService},
    web::templates::{
        event_register::{holds_a_seat, RegisterPageQuery, SignedInMember},
        BaseContext, HtmlTemplate,
    },
};

#[derive(Template)]
#[template(path = "events/class_register.html")]
pub struct ClassRegisterTemplate {
    pub base: BaseContext,
    pub series_id: String,
    pub title: String,
    /// Sanitized safe-subset HTML from the shared Markdown pipeline — the
    /// raw Markdown never reaches the template. See `EventRegisterTemplate`.
    pub description_html: String,
    /// Rendered "6 sessions, Tuesdays from …" line: how many sessions are
    /// still to come and when the first of them is. A guest buying a pass
    /// is buying those sessions, so that is what the page states.
    pub schedule: String,
    /// Every remaining session, wall-clock in the class's own zone.
    pub upcoming: Vec<String>,
    pub location: Option<String>,
    /// `"Free"` rather than `"$0.00"` — a zero price is free, and
    /// rendering it as money reads like a bug.
    pub guest_price_display: String,
    /// Member price + login link, shown only when it differs, so a member
    /// about to overpay is told before paying.
    pub member_price_display: Option<String>,
    /// Rendered "N places remaining", or `None` for an uncapped class.
    pub places_remaining: Option<String>,
    pub is_full: bool,
    /// True when every session has already happened — there is nothing
    /// left to sell, so the form is replaced with a notice.
    pub is_over: bool,
    pub captcha_site_key: Option<String>,
    pub just_registered: bool,
    /// The visitor's member session, when they have one — see
    /// [`SignedInMember`]. `None` renders today's guest page.
    pub signed_in: Option<SignedInMember>,
}

// Granular state, same as `event_register_page` — see the note there.
#[allow(clippy::too_many_arguments)]
pub async fn class_register_page(
    State(event_repo): State<Arc<dyn EventRepository>>,
    State(series_repo): State<Arc<dyn EventSeriesRepository>>,
    State(enrollment_repo): State<Arc<dyn SeriesEnrollmentRepository>>,
    State(settings_service): State<Arc<SettingsService>>,
    State(auth_service): State<Arc<AuthService>>,
    State(member_repo): State<Arc<dyn MemberRepository>>,
    State(csrf_service): State<Arc<CsrfService>>,
    jar: CookieJar,
    Path(series_id): Path<Uuid>,
    Query(query): Query<RegisterPageQuery>,
) -> Response {
    let Some((series, occurrences)) = load_enrollable(&*event_repo, &*series_repo, series_id).await
    else {
        return not_found();
    };

    let now = chrono::Utc::now();
    let upcoming: Vec<_> = occurrences.iter().filter(|e| e.start_utc() > now).collect();
    let prototype = occurrences
        .first()
        .expect("enrollable series has an occurrence");

    // Capacity is the held-enrollment count — the same predicate the claim
    // enforces — so the page and the endpoint agree about "full".
    let held = enrollment_repo.count_held(series.id).await.unwrap_or(0);
    let places_remaining = series
        .max_enrollments
        .map(|max| (i64::from(max) - held).max(0));

    // Only render the challenge widget when a provider is configured: the
    // endpoint fails closed on a missing token, so a page with no widget
    // against an active provider would be a form nobody can submit.
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

    // Resolved after the enrollability 404 and fail-open, exactly as the
    // event page does it — see `event_register_page`.
    let (base, signed_in) = match optional_member(&auth_service, &*member_repo, &jar).await {
        Some((user, session)) => {
            let already_registered = holds_a_seat(
                enrollment_repo
                    .find(series.id, &Attendee::Member(user.member.id))
                    .await
                    .ok()
                    .flatten()
                    .map(|e| e.status),
            );
            (
                BaseContext::for_member(&csrf_service, &user, &session).await,
                Some(SignedInMember {
                    price_display: if series.is_paid_class() {
                        format!("${:.2}", series.member_price_cents as f64 / 100.0)
                    } else {
                        "Free".to_string()
                    },
                    already_registered,
                }),
            )
        }
        None => (BaseContext::for_anon(), None),
    };

    let template = ClassRegisterTemplate {
        base,
        series_id: series.id.to_string(),
        title: prototype.title.clone(),
        description_html: crate::util::markdown::render_markdown(&prototype.description),
        schedule: match upcoming.first() {
            Some(first) => format!(
                "{} session{} remaining, starting {} {}",
                upcoming.len(),
                if upcoming.len() == 1 { "" } else { "s" },
                first.start_time.format("%A, %B %-d, %Y at %-I:%M %p"),
                first.zone_abbr(),
            ),
            None => "This class has finished — no sessions remain.".to_string(),
        },
        upcoming: upcoming
            .iter()
            .map(|e| {
                format!(
                    "{} {}",
                    e.start_time.format("%a %b %-d, %Y · %-I:%M %p"),
                    e.zone_abbr(),
                )
            })
            .collect(),
        location: prototype.location.clone(),
        guest_price_display: if series.is_paid_for_guests() {
            format!("${:.2}", series.guest_price_cents as f64 / 100.0)
        } else {
            "Free".to_string()
        },
        member_price_display: (series.member_price_cents != series.guest_price_cents).then(|| {
            if series.is_paid_class() {
                format!("${:.2}", series.member_price_cents as f64 / 100.0)
            } else {
                "free".to_string()
            }
        }),
        places_remaining: places_remaining
            .map(|n| format!("{} place{} remaining", n, if n == 1 { "" } else { "s" })),
        is_full: places_remaining == Some(0),
        is_over: upcoming.is_empty(),
        captcha_site_key,
        just_registered: query.registered.is_some(),
        signed_in,
    };
    HtmlTemplate(template).into_response()
}

/// The series and its occurrences, but only when the public may enroll.
///
/// One rule, one home: [`crate::domain::EventSeries::publicly_enrollable`]
/// read against an occurrence, since a series row carries no visibility of
/// its own. A series with no occurrences at all is not enrollable —
/// there'd be nothing to sell.
pub async fn load_enrollable(
    event_repo: &dyn EventRepository,
    series_repo: &dyn EventSeriesRepository,
    series_id: Uuid,
) -> Option<(crate::domain::EventSeries, Vec<crate::domain::Event>)> {
    let series = match series_repo.find_by_id(series_id).await {
        Ok(Some(s)) => s,
        Ok(None) => return None,
        Err(e) => {
            tracing::error!("Class page lookup failed for series {}: {}", series_id, e);
            return None;
        }
    };
    let occurrences = event_repo
        .list_series_occurrences(series_id)
        .await
        .unwrap_or_default();
    let first = occurrences.first()?;
    series.publicly_enrollable(first).then_some(())?;
    Some((series, occurrences))
}

/// The one response a non-enrollable class, a bad id, and a lookup error
/// all share.
fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "Not Found").into_response()
}
