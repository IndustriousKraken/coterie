use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
    Extension,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    api::middleware::auth::{CurrentUser, SessionInfo},
    auth::CsrfService,
    domain::AttendanceStatus,
    repository::EventRepository,
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
    pub show_past: Option<bool>,
}

pub async fn events_list_api(
    State(event_repo): State<Arc<dyn EventRepository>>,
    Extension(current_user): Extension<CurrentUser>,
    Query(query): Query<EventsListQuery>,
) -> impl IntoResponse {
    let member_id = current_user.member.id;

    // Get upcoming events (past events not currently supported)
    let events = event_repo.list_upcoming(50).await.unwrap_or_default();

    let now = chrono::Utc::now();

    // Filter events by type (past events not currently supported by repository)
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
            .get_member_attendance_status(event.id, member_id)
            .await
            .ok()
            .flatten();

        let rsvp_button = if is_past {
            String::new()
        } else {
            render_rsvp_button(&event.id.to_string(), rsvp_status.as_ref())
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
fn render_rsvp_button(event_id: &str, status: Option<&AttendanceStatus>) -> String {
    match status {
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
            format!(
                r#"<div class="text-right">
                    <button hx-post="/portal/api/events/{}/rsvp"
                            hx-swap="outerHTML"
                            hx-target="closest div.text-right"
                            class="px-4 py-2 bg-blue-600 text-white text-sm rounded-md hover:bg-blue-700">
                        RSVP
                    </button>
                </div>"#,
                event_id
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
fn render_rsvp_error(event_id: &str, status: Option<&AttendanceStatus>, error: &str) -> String {
    let button = render_rsvp_button(event_id, status);
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

/// Handle RSVP to an event
pub async fn rsvp_event(
    State(event_repo): State<Arc<dyn EventRepository>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(event_id): Path<Uuid>,
) -> impl IntoResponse {
    let member_id = current_user.member.id;

    // Register attendance
    if let Err(e) = event_repo.register_attendance(event_id, member_id).await {
        // Registration failed: member is still unregistered, so re-render the
        // RSVP button (unchanged state) alongside the error.
        return axum::response::Html(render_rsvp_error(
            &event_id.to_string(),
            None,
            &e.to_string(),
        ));
    }

    // Return updated button
    axum::response::Html(render_rsvp_button(
        &event_id.to_string(),
        Some(&AttendanceStatus::Registered),
    ))
}

/// Handle cancel RSVP
pub async fn cancel_rsvp_event(
    State(event_repo): State<Arc<dyn EventRepository>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(event_id): Path<Uuid>,
) -> impl IntoResponse {
    let member_id = current_user.member.id;

    // Cancel attendance
    if let Err(e) = event_repo.cancel_attendance(event_id, member_id).await {
        // Cancel failed: member is still attending, so re-render the Cancel
        // button (unchanged state) alongside the error. ponytail: shows the
        // "Registered" label even if the member was waitlisted — the retry
        // action (cancel) is identical for both; re-fetch the exact status if
        // that cosmetic label ever matters.
        return axum::response::Html(render_rsvp_error(
            &event_id.to_string(),
            Some(&AttendanceStatus::Registered),
            &e.to_string(),
        ));
    }

    // Return updated button (shows RSVP button again)
    axum::response::Html(render_rsvp_button(&event_id.to_string(), None))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        ];
        for status in statuses {
            let fragment = render_rsvp_button("evt-1", status.as_ref());
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
        let register_err = render_rsvp_error("evt-1", None, "boom");
        // Cancel failure: unchanged state is "attending" -> Cancel button.
        let cancel_err = render_rsvp_error("evt-1", Some(&AttendanceStatus::Registered), "boom");

        for (fragment, endpoint) in [(&register_err, "rsvp"), (&cancel_err, "cancel")] {
            assert!(fragment.trim_start().starts_with(r#"<div class="text-right">"#));
            assert!(fragment.contains(r#"hx-target="closest div.text-right""#));
            assert!(fragment.contains(&format!("/portal/api/events/evt-1/{endpoint}")));
            assert!(fragment.contains("Error: boom"));
        }
    }

    // Error strings are HTML-escaped so a failure message can't inject markup.
    #[test]
    fn rsvp_error_fragment_escapes_message() {
        let fragment = render_rsvp_error("evt-1", None, "<script>x</script>");
        assert!(!fragment.contains("<script>"));
        assert!(fragment.contains("&lt;script&gt;"));
    }
}
