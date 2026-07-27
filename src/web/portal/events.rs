use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    Extension,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    api::middleware::auth::{CurrentUser, SessionInfo},
    auth::CsrfService,
    domain::{AttendanceStatus, Attendee},
    repository::EventRepository,
    service::event_registration_service::{EventRegistrationService, RegistrationOutcome},
    web::templates::{BaseContext, HtmlTemplate},
};

#[derive(Template)]
#[template(path = "portal/events.html")]
pub struct EventsTemplate {
    pub base: BaseContext,
}

pub async fn events_page(
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session): Extension<SessionInfo>,
) -> impl IntoResponse {
    let template = EventsTemplate {
        base: BaseContext::for_member(&csrf_service, &current_user, &session).await,
    };

    HtmlTemplate(template)
}

// API endpoint for events list (for events page)
#[derive(Debug, Deserialize)]
pub struct EventsListQuery {
    pub event_type: Option<String>,
    // HTML checkbox serializes as `show_past=on` when checked, absent when not — a
    // bare String parses both; `Option<bool>` 400s on "on" (serde_urlencoded).
    pub show_past: Option<String>,
}

pub async fn events_list_api(
    State(event_repo): State<Arc<dyn EventRepository>>,
    Extension(current_user): Extension<CurrentUser>,
    Query(query): Query<EventsListQuery>,
) -> impl IntoResponse {
    let member_id = current_user.member.id;

    // "Show past events": when checked, list every event (`list` returns all
    // events — public AND members-only — newest-first); otherwise only upcoming.
    // The checkbox serializes as `show_past=on`.
    let show_past = query.show_past.is_some();

    let now = chrono::Utc::now();

    let mut events = if show_past {
        event_repo.list(200, 0).await.unwrap_or_default()
    } else {
        event_repo.list_upcoming(50).await.unwrap_or_default()
    };

    // Display order: upcoming soonest-first, then past most-recent-first, so a
    // combined view reads "what's coming" then "what just happened". Compare on
    // the derived UTC instant (`start_utc`), never the naive wall-clock, to stay
    // correct for non-UTC orgs.
    events.sort_by(|a, b| {
        let (au, bu) = (a.start_utc(), b.start_utc());
        match (au > now, bu > now) {
            (true, true) => au.cmp(&bu),
            (false, false) => bu.cmp(&au),
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
        }
    });

    // Filter events by type.
    let filtered_events: Vec<_> = events
        .into_iter()
        .filter(|e| {
            // Filter by type
            if let Some(ref event_type) = query.event_type {
                if !event_type.is_empty() && format!("{:?}", e.event_type) != *event_type {
                    return false;
                }
            }
            true
        })
        .collect();

    if filtered_events.is_empty() {
        return axum::response::Html(
            r#"<div class="bg-white rounded-lg shadow-sm p-6 text-center text-gray-500">
                No events found matching your criteria
            </div>"#
                .to_string(),
        );
    }

    let mut html = String::new();
    html.push_str(r#"<div class="space-y-4">"#);

    for event in filtered_events {
        let is_past = event.start_utc() <= now;
        // Label the wall-clock with the event's zone so a remote member
        // knows which local time it is (server-rendered — no browser
        // conversion like the public JSON/iCal path gets).
        let tz_abbr = event.zone_abbr();
        let type_badge_color = match format!("{:?}", event.event_type).as_str() {
            "Meeting" => "bg-blue-100 text-blue-800",
            "Workshop" => "bg-purple-100 text-purple-800",
            "CTF" => "bg-red-100 text-red-800",
            "Social" => "bg-green-100 text-green-800",
            "Training" => "bg-yellow-100 text-yellow-800",
            _ => "bg-gray-100 text-gray-800",
        };

        // Check member's RSVP status for this event
        let rsvp_status = event_repo
            .attendance_status(event.id, &Attendee::Member(member_id))
            .await
            .ok()
            .flatten();

        let rsvp_button = if is_past {
            String::new()
        } else {
            render_rsvp_button(
                &event.id.to_string(),
                rsvp_status.as_ref(),
                event.member_price_cents,
            )
        };

        let image_html = event.image_url.as_ref().map(|url| {
            format!(r#"<div class="bg-gray-100 rounded-t-lg -mt-6 -mx-6 mb-4 overflow-hidden" style="width: calc(100% + 3rem);"><img src="/{}" alt="" class="w-full h-40 object-contain"></div>"#, crate::web::escape_html(url))
        }).unwrap_or_default();

        html.push_str(&format!(
            r#"<div class="bg-white rounded-lg shadow-sm p-6 {}">
                {}
                <div class="flex justify-between items-start">
                    <div>
                        <div class="flex items-center gap-2 mb-2">
                            <span class="px-2 py-1 text-xs font-medium rounded {}">{:?}</span>
                            {}
                        </div>
                        <h3 class="text-lg font-semibold text-gray-900">{}</h3>
                        <p class="text-sm text-gray-600 mt-1">{}</p>
                        <div class="mt-2 text-sm text-gray-500">
                            <p>{} at {}</p>
                            {}
                        </div>

                    </div>
                    {}
                </div>
            </div>"#,
            if is_past { "opacity-60" } else { "" },
            image_html,
            type_badge_color,
            event.event_type,
            if is_past {
                r#"<span class="text-xs text-gray-500">Past event</span>"#
            } else {
                ""
            },
            crate::web::escape_html(&event.title),
            crate::web::escape_html(&event.description),
            event.start_time.format("%B %d, %Y"),
            format!("{} {}", event.start_time.format("%l:%M %p"), tz_abbr),
            event
                .location
                .map(|l| format!(r#"<p>Location: {}</p>"#, crate::web::escape_html(&l)))
                .unwrap_or_default(),
            rsvp_button,
        ));
    }

    html.push_str("</div>");
    axum::response::Html(html)
}

/// Render the appropriate RSVP button based on current status.
///
/// The fragment is self-wrapped in a `<div class="text-right">` root so it
/// matches the `hx-target="closest div.text-right"` selector: each outerHTML
/// swap replaces and re-emits the same targeted element, so repeated
/// RSVP <-> cancel toggles keep resolving the target instead of failing
/// silently after the first swap.
pub(crate) fn render_rsvp_button(
    event_id: &str,
    status: Option<&AttendanceStatus>,
    price_cents: i64,
) -> String {
    let is_paid = price_cents > 0;
    match status {
        // A paid seat gets no self-service cancel button: cancelling
        // would drop the seat while the charge stood, and refunds are
        // an operator action. Free RSVPs keep today's toggle.
        Some(AttendanceStatus::Registered) if is_paid => r#"<div class="text-right">
                    <div class="flex flex-col items-end gap-2">
                        <span class="text-sm text-green-600 font-medium">You're attending — paid</span>
                        <span class="text-xs text-gray-500">Need a refund? Contact an organizer.</span>
                    </div>
                </div>"#
            .to_string(),
        Some(AttendanceStatus::PendingPayment) => {
            format!(
                r#"<div class="text-right">
                    <div class="flex flex-col items-end gap-2">
                        <span class="text-sm text-yellow-600 font-medium">Awaiting payment</span>
                        <button hx-post="/portal/api/events/{}/rsvp"
                                hx-swap="outerHTML"
                                hx-target="closest div.text-right"
                                class="px-4 py-2 bg-blue-600 text-white text-sm rounded-md hover:bg-blue-700">
                            Complete payment
                        </button>
                    </div>
                </div>"#,
                event_id
            )
        }
        Some(AttendanceStatus::Registered) => {
            format!(
                r#"<div class="text-right">
                    <div class="flex flex-col items-end gap-2">
                        <span class="text-sm text-green-600 font-medium">You're attending</span>
                        <button hx-post="/portal/api/events/{}/cancel"
                                hx-swap="outerHTML"
                                hx-target="closest div.text-right"
                                class="px-3 py-1 text-sm text-gray-600 border border-gray-300 rounded-md hover:bg-gray-50">
                            Cancel RSVP
                        </button>
                    </div>
                </div>"#,
                event_id
            )
        }
        Some(AttendanceStatus::Waitlisted) => {
            format!(
                r#"<div class="text-right">
                    <div class="flex flex-col items-end gap-2">
                        <span class="text-sm text-yellow-600 font-medium">On waitlist</span>
                        <button hx-post="/portal/api/events/{}/cancel"
                                hx-swap="outerHTML"
                                hx-target="closest div.text-right"
                                class="px-3 py-1 text-sm text-gray-600 border border-gray-300 rounded-md hover:bg-gray-50">
                            Leave waitlist
                        </button>
                    </div>
                </div>"#,
                event_id
            )
        }
        Some(AttendanceStatus::Cancelled) | None => {
            // The label says which this control is — an RSVP, or a trip
            // to a payment page — so a member knows before clicking.
            let label = if is_paid {
                format!("Register — ${:.2}", price_cents as f64 / 100.0)
            } else {
                "RSVP".to_string()
            };
            format!(
                r#"<div class="text-right">
                    <button hx-post="/portal/api/events/{}/rsvp"
                            hx-swap="outerHTML"
                            hx-target="closest div.text-right"
                            class="px-4 py-2 bg-blue-600 text-white text-sm rounded-md hover:bg-blue-700">
                        {}
                    </button>
                </div>"#,
                event_id, label
            )
        }
    }
}

/// Render an RSVP error fragment.
///
/// On a failed register/cancel the state is unchanged, so we re-render the
/// button for that unchanged `status` (giving the member a working retry) and
/// inject the error message just inside the same `div.text-right` root. This
/// keeps the `hx-target="closest div.text-right"` target alive after the swap
/// instead of leaving a buttonless error message that freezes the UI until a
/// full page refresh.
fn render_rsvp_error(
    event_id: &str,
    status: Option<&AttendanceStatus>,
    price_cents: i64,
    error: &str,
) -> String {
    let button = render_rsvp_button(event_id, status, price_cents);
    // render_rsvp_button always emits a `<div class="text-right">` root
    // (asserted by every_rsvp_fragment_root_matches_hx_target), so inject the
    // error span right after that opening tag to share the target wrapper.
    button.replacen(
        r#"<div class="text-right">"#,
        &format!(
            r#"<div class="text-right"><p class="text-red-600 text-sm mb-2">Error: {}</p>"#,
            crate::web::escape_html(error)
        ),
        1,
    )
}

/// Handle RSVP to an event.
///
/// Free events behave exactly as before. A paid event routes through
/// `EventRegistrationService`, which claims the seat and returns a
/// Checkout URL — handed back as an `HX-Redirect` so HTMX sends the
/// browser to Stripe instead of swapping a fragment. The member is not
/// `Registered` until the completion webhook says so.
pub async fn rsvp_event(
    State(event_repo): State<Arc<dyn EventRepository>>,
    State(registration_service): State<Arc<EventRegistrationService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(event_id): Path<Uuid>,
) -> Response {
    let event = match event_repo.find_by_id(event_id).await {
        Ok(Some(e)) => e,
        Ok(None) => {
            return axum::response::Html(render_rsvp_error(
                &event_id.to_string(),
                None,
                0,
                "Event not found",
            ))
            .into_response()
        }
        Err(e) => {
            return axum::response::Html(render_rsvp_error(
                &event_id.to_string(),
                None,
                0,
                &e.to_string(),
            ))
            .into_response()
        }
    };
    let price = event.member_price_cents;

    match registration_service
        .register(&current_user.member, &event)
        .await
    {
        Ok(RegistrationOutcome::Registered) => axum::response::Html(render_rsvp_button(
            &event_id.to_string(),
            Some(&AttendanceStatus::Registered),
            price,
        ))
        .into_response(),
        Ok(RegistrationOutcome::Checkout { url }) => {
            // 200 + HX-Redirect: HTMX navigates the whole page. The
            // body is the pending-payment fragment so a non-HTMX
            // client still sees the seat's real state.
            (
                [("HX-Redirect", url)],
                axum::response::Html(render_rsvp_button(
                    &event_id.to_string(),
                    Some(&AttendanceStatus::PendingPayment),
                    price,
                )),
            )
                .into_response()
        }
        Err(e) => {
            // Registration failed: state is unchanged, so re-render the
            // register button (working retry) alongside the error.
            axum::response::Html(render_rsvp_error(
                &event_id.to_string(),
                None,
                price,
                &e.to_string(),
            ))
            .into_response()
        }
    }
}

/// Handle cancel RSVP
pub async fn cancel_rsvp_event(
    State(event_repo): State<Arc<dyn EventRepository>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(event_id): Path<Uuid>,
) -> Response {
    let member_id = current_user.member.id;
    let price = event_repo
        .find_by_id(event_id)
        .await
        .ok()
        .flatten()
        .map(|e| e.member_price_cents)
        .unwrap_or(0);

    // Self-service cancel on a paid event would surrender the seat while
    // the charge stood. Releasing the money is an operator action
    // (refund), which cancels the seat as a side effect.
    if price > 0 {
        return axum::response::Html(render_rsvp_error(
            &event_id.to_string(),
            Some(&AttendanceStatus::Registered),
            price,
            "Paid registrations can't be cancelled here — contact an organizer for a refund.",
        ))
        .into_response();
    }

    // Cancel attendance
    if let Err(e) = event_repo
        .cancel_attendance(event_id, &Attendee::Member(member_id))
        .await
    {
        // Cancel failed: member is still attending, so re-render the Cancel
        // button (unchanged state) alongside the error. ponytail: shows the
        // "Registered" label even if the member was waitlisted — the retry
        // action (cancel) is identical for both; re-fetch the exact status if
        // that cosmetic label ever matters.
        return axum::response::Html(render_rsvp_error(
            &event_id.to_string(),
            Some(&AttendanceStatus::Registered),
            price,
            &e.to_string(),
        ))
        .into_response();
    }

    // Return updated button (shows RSVP button again)
    axum::response::Html(render_rsvp_button(&event_id.to_string(), None, price)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        routing::get,
        Router,
    };
    use chrono::Utc;
    use tower::ServiceExt;

    use crate::{
        domain::{CreateMemberRequest, Event, EventType, EventVisibility, Member},
        repository::{MemberRepository, SqliteEventRepository, SqliteMemberRepository},
    };

    async fn migrated_pool() -> sqlx::SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    // Regression: the "Show past events" checkbox serializes as `show_past=on`.
    // When `EventsListQuery.show_past` was `Option<bool>`, serde_urlencoded
    // could not parse "on", so the `Query` extractor 400'd and the
    // member-content events fragment (`GET /portal/api/events/list`) broke
    // whenever the box was checked. It must now return 200 and render the list.
    #[tokio::test]
    async fn show_past_on_returns_list_fragment() {
        let pool = migrated_pool().await;

        let member_repo: Arc<dyn MemberRepository> =
            Arc::new(SqliteMemberRepository::new(pool.clone()));
        let member: Member = member_repo
            .create(CreateMemberRequest {
                email: "member@example.com".to_string(),
                username: "member".to_string(),
                full_name: "Member".to_string(),
                password: "p4ssword_long_enough".to_string(),
                membership_type_id: None,
                ..Default::default()
            })
            .await
            .unwrap();

        let event_repo: Arc<dyn EventRepository> =
            Arc::new(SqliteEventRepository::new(pool.clone()));
        let now = Utc::now();
        event_repo
            .create(Event {
                id: Uuid::new_v4(),
                title: "Upcoming Meetup".to_string(),
                description: "Body".to_string(),
                event_type: EventType::Social,
                event_type_id: None,
                visibility: EventVisibility::Public,
                start_time: now + chrono::Duration::days(7),
                end_time: None,
                timezone: "UTC".to_string(),
                location: None,
                max_attendees: None,
                rsvp_required: false,
                member_price_cents: 0,
                guest_price_cents: 0,
                guest_registration_enabled: false,
                image_url: None,
                created_by: member.id,
                created_at: now,
                updated_at: now,
                series_id: None,
                occurrence_index: None,
            })
            .await
            .unwrap();

        let app = Router::new()
            .route("/portal/api/events/list", get(events_list_api))
            .layer(Extension(CurrentUser { member }))
            .with_state(event_repo);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/portal/api/events/list?show_past=on")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "checkbox `show_past=on` must parse, not 400"
        );

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(
            body.contains("Upcoming Meetup"),
            "list fragment should render the upcoming event, got: {body}"
        );
    }

    // Regression for #101: every RSVP fragment must be rooted in the same
    // `div.text-right` element the buttons target with
    // `hx-target="closest div.text-right"`. If any status returns a different
    // root, the first outerHTML swap replaces the target wrapper and the next
    // click can no longer resolve it — so the UI freezes until a page refresh.
    #[test]
    fn every_rsvp_fragment_root_matches_hx_target() {
        let statuses = [
            None,
            Some(AttendanceStatus::Cancelled),
            Some(AttendanceStatus::Registered),
            Some(AttendanceStatus::Waitlisted),
            Some(AttendanceStatus::PendingPayment),
        ];
        for status in statuses {
            let fragment = render_rsvp_button("evt-1", status.as_ref(), 0);
            let root = fragment.trim_start();
            assert!(
                root.starts_with(r#"<div class="text-right">"#),
                "fragment for {status:?} must be rooted in div.text-right, got: {root}"
            );
            // The swap target the buttons name must exist in the fragment they
            // re-emit, so a subsequent toggle can still resolve it.
            assert!(fragment.contains(r#"hx-target="closest div.text-right""#));
        }
    }

    // Error responses must keep the same `div.text-right` root AND re-emit a
    // working button, so a failed register/cancel doesn't strand the member
    // with a buttonless error that only a page refresh can clear.
    #[test]
    fn rsvp_error_fragment_keeps_target_and_retry_button() {
        // Register failure: unchanged state is "not attending" -> RSVP button.
        let register_err = render_rsvp_error("evt-1", None, 0, "boom");
        // Cancel failure: unchanged state is "attending" -> Cancel button.
        let cancel_err = render_rsvp_error("evt-1", Some(&AttendanceStatus::Registered), 0, "boom");

        for (fragment, endpoint) in [(&register_err, "rsvp"), (&cancel_err, "cancel")] {
            assert!(fragment
                .trim_start()
                .starts_with(r#"<div class="text-right">"#));
            assert!(fragment.contains(r#"hx-target="closest div.text-right""#));
            assert!(fragment.contains(&format!("/portal/api/events/evt-1/{endpoint}")));
            assert!(fragment.contains("Error: boom"));
        }
    }

    // The control's label tells a member which it is BEFORE they click:
    // an RSVP that registers them, or a trip to a payment page.
    #[test]
    fn paid_event_button_shows_the_price() {
        let free = render_rsvp_button("evt-1", None, 0);
        assert!(
            free.contains(">\n                        RSVP\n"),
            "got: {free}"
        );
        assert!(!free.contains('$'));

        let paid = render_rsvp_button("evt-1", None, 3000);
        assert!(paid.contains("Register — $30.00"), "got: {paid}");
        assert!(paid.contains("/portal/api/events/evt-1/rsvp"));
    }

    // A seat held at Stripe reads as awaiting payment and offers a way
    // back to the same checkout, not a second registration.
    #[test]
    fn pending_payment_fragment_offers_completion() {
        let f = render_rsvp_button("evt-1", Some(&AttendanceStatus::PendingPayment), 3000);
        assert!(f.trim_start().starts_with(r#"<div class="text-right">"#));
        assert!(f.contains("Awaiting payment"));
        assert!(f.contains("Complete payment"));
        assert!(f.contains("/portal/api/events/evt-1/rsvp"));
    }

    // Cancelling a paid seat would surrender it while the charge stood,
    // so the member-facing control simply isn't offered.
    #[test]
    fn paid_registered_fragment_has_no_self_service_cancel() {
        let paid = render_rsvp_button("evt-1", Some(&AttendanceStatus::Registered), 3000);
        assert!(!paid.contains("/cancel"), "got: {paid}");
        assert!(paid.contains("You're attending — paid"));

        // Free events keep today's toggle.
        let free = render_rsvp_button("evt-1", Some(&AttendanceStatus::Registered), 0);
        assert!(free.contains("/portal/api/events/evt-1/cancel"));
    }

    // Error strings are HTML-escaped so a failure message can't inject markup.
    #[test]
    fn rsvp_error_fragment_escapes_message() {
        let fragment = render_rsvp_error("evt-1", None, 0, "<script>x</script>");
        assert!(!fragment.contains("<script>"));
        assert!(fragment.contains("&lt;script&gt;"));
    }
}
