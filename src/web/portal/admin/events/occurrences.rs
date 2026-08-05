use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Multipart, Path, State},
    response::IntoResponse,
    Extension,
};
use serde::Deserialize;

use crate::{
    api::middleware::auth::{CurrentUser, SessionInfo},
    auth::CsrfService,
    domain::OccurrenceOverride,
    repository::{EventRepository, EventSeriesRepository},
    service::event_admin_service::EventAdminService,
    web::portal::admin::partials,
    web::templates::{BaseContext, HtmlTemplate},
};

use super::RosterRow;

// =====================================================================
// Per-occurrence exception handlers
//
// These three POST handlers + one GET sit under /portal/admin/events/
// series/:id/occurrences/:index/ and back the cancel / edit-just-this
// / restore affordances on the event-series detail page.

#[derive(Template)]
#[template(path = "admin/_event_occurrence_row.html")]
pub struct EventOccurrenceRowTemplate {
    pub row: OccurrenceRowInfo,
    pub csrf_token: String,
}

/// One row in the occurrences list rendered on the event-series detail
/// page. Carries everything the row needs to render its own action
/// buttons — past/future, cancelled/overridden/normal.
#[derive(Clone)]
pub struct OccurrenceRowInfo {
    pub series_id: String,
    pub occurrence_index: i32,
    pub event_id: Option<String>,
    pub title: String,
    pub start_time: String,
    pub location: Option<String>,
    pub is_past: bool,
    pub state: &'static str,
    pub reason: Option<String>,
}

impl OccurrenceRowInfo {
    pub fn from_active(event: &crate::domain::Event, now: chrono::DateTime<chrono::Utc>) -> Self {
        Self {
            series_id: event.series_id.expect("series occurrence").to_string(),
            occurrence_index: event.occurrence_index.unwrap_or(0),
            event_id: Some(event.id.to_string()),
            title: event.title.clone(),
            start_time: event.start_time.format("%b %d, %Y %H:%M").to_string(),
            location: event.location.clone(),
            is_past: event.start_utc() <= now,
            state: "active",
            reason: None,
        }
    }
}

#[derive(Template)]
#[template(path = "admin/event_occurrence_override_form.html")]
pub struct EventOccurrenceOverrideFormTemplate {
    pub series_id: String,
    pub occurrence_index: i32,
    pub event: OverrideFormEvent,
    pub csrf_token: String,
}

pub struct OverrideFormEvent {
    pub title: String,
    pub description: String,
    pub start_time_input: String,
    pub end_time_input: Option<String>,
    pub location: Option<String>,
    pub max_attendees: Option<i32>,
    pub rsvp_required: bool,
    pub image_url: Option<String>,
}

/// GET — render the override form for an occurrence. Returns an HTMX
/// fragment the caller swaps into a modal container.
pub async fn admin_occurrence_override_form(
    State(event_repo): State<Arc<dyn EventRepository>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(_current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Path((series_id, idx)): Path<(String, i32)>,
) -> impl IntoResponse {
    let series_uuid = match uuid::Uuid::parse_str(&series_id) {
        Ok(u) => u,
        Err(_) => {
            return partials::admin_alert("error", "Invalid series ID", false).into_response()
        }
    };
    let event = match event_repo.find_by_series_and_index(series_uuid, idx).await {
        Ok(Some(e)) => e,
        Ok(None) => {
            return partials::admin_alert("error", "Occurrence not found", false).into_response()
        }
        Err(_) => {
            return partials::admin_alert("error", "Error loading occurrence", false)
                .into_response()
        }
    };

    let token = csrf_service
        .generate_token(&session_info.session_id)
        .await
        .unwrap_or_default();

    let template = EventOccurrenceOverrideFormTemplate {
        series_id,
        occurrence_index: idx,
        event: OverrideFormEvent {
            title: event.title,
            description: event.description,
            start_time_input: event.start_time.format("%Y-%m-%dT%H:%M").to_string(),
            end_time_input: event
                .end_time
                .map(|t| t.format("%Y-%m-%dT%H:%M").to_string()),
            location: event.location,
            max_attendees: event.max_attendees,
            rsvp_required: event.rsvp_required,
            image_url: event.image_url,
        },
        csrf_token: token,
    };
    HtmlTemplate(template).into_response()
}

#[derive(Deserialize, Default)]
pub struct CancelOccurrenceForm {
    pub reason: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub csrf_token: String,
}

/// POST — cancel a single occurrence in a series. The HX-Prompt header
/// (sent by HTMX's `hx-prompt` attribute) carries the optional reason.
pub async fn admin_cancel_event_occurrence(
    State(event_admin_service): State<Arc<EventAdminService>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Path((series_id, idx)): Path<(String, i32)>,
    headers: axum::http::HeaderMap,
    axum::Form(form): axum::Form<CancelOccurrenceForm>,
) -> impl IntoResponse {
    let series_uuid = match uuid::Uuid::parse_str(&series_id) {
        Ok(u) => u,
        Err(_) => {
            return partials::admin_alert("error", "Invalid series ID", false).into_response()
        }
    };

    // hx-prompt sends the typed value as the HX-Prompt header; fall back
    // to the form field if a non-HTMX client posts directly.
    let reason = headers
        .get("HX-Prompt")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .or(form.reason.filter(|s| !s.is_empty()));

    match event_admin_service
        .cancel_event_occurrence(current_user.member.id, series_uuid, idx, reason.clone())
        .await
    {
        // Cancelling withdraws the occurrence from the public feed, so a
        // failure to tell the public site takes priority over the row
        // swap: the occurrence is cancelled here either way, and the
        // admin has to know the other system still shows it. Autoreload
        // brings the row up to date once they've read it.
        Ok(public_site) if public_site.is_failed() => partials::admin_alert(
            "warning",
            &format!(
                "Occurrence cancelled. {}",
                public_site.admin_note_deleted().unwrap_or_default()
            ),
            true,
        )
        .into_response(),
        Ok(_) => {
            let token = csrf_service
                .generate_token(&session_info.session_id)
                .await
                .unwrap_or_default();
            HtmlTemplate(EventOccurrenceRowTemplate {
                row: OccurrenceRowInfo {
                    series_id,
                    occurrence_index: idx,
                    event_id: None,
                    title: String::new(),
                    start_time: String::new(),
                    location: None,
                    is_past: false,
                    state: "cancelled",
                    reason,
                },
                csrf_token: token,
            })
            .into_response()
        }
        Err(e) => partials::admin_alert(
            "error",
            &format!("Error cancelling occurrence: {}", e),
            false,
        )
        .into_response(),
    }
}

/// POST — apply per-occurrence overrides. Multipart form parsed into
/// `OccurrenceOverride`.
pub async fn admin_override_event_occurrence(
    State(event_admin_service): State<Arc<EventAdminService>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Path((series_id, idx)): Path<(String, i32)>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let series_uuid = match uuid::Uuid::parse_str(&series_id) {
        Ok(u) => u,
        Err(_) => {
            return partials::admin_alert("error", "Invalid series ID", false).into_response()
        }
    };

    let mut overrides = OccurrenceOverride::default();
    let mut reason: Option<String> = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        let val = field.text().await.unwrap_or_default();
        match name.as_str() {
            "csrf_token" => {}
            "reason" if !val.is_empty() => reason = Some(val),
            "title" if !val.is_empty() => overrides.title = Some(val),
            "description" if !val.is_empty() => overrides.description = Some(val),
            "start_time" => {
                if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(&val, "%Y-%m-%dT%H:%M") {
                    overrides.start_time =
                        Some(chrono::DateTime::from_naive_utc_and_offset(dt, chrono::Utc));
                }
            }
            "end_time" => {
                if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(&val, "%Y-%m-%dT%H:%M") {
                    overrides.end_time =
                        Some(chrono::DateTime::from_naive_utc_and_offset(dt, chrono::Utc));
                }
            }
            "location" if !val.is_empty() => overrides.location = Some(val),
            "max_attendees" => {
                if let Ok(n) = val.parse::<i32>() {
                    overrides.max_attendees = Some(n);
                }
            }
            "rsvp_required" => overrides.rsvp_required = Some(true),
            "image_url" if !val.is_empty() => overrides.image_url = Some(val),
            _ => {}
        }
    }

    match event_admin_service
        .override_event_occurrence(current_user.member.id, series_uuid, idx, overrides, reason)
        .await
    {
        Ok((_, public_site)) if public_site.is_failed() => partials::admin_alert(
            "warning",
            &format!(
                "Occurrence overridden. {}",
                public_site.admin_note().unwrap_or_default()
            ),
            true,
        )
        .into_response(),
        Ok((event, _)) => {
            let token = csrf_service
                .generate_token(&session_info.session_id)
                .await
                .unwrap_or_default();
            let now = chrono::Utc::now();
            let mut row = OccurrenceRowInfo::from_active(&event, now);
            row.state = "overridden";
            HtmlTemplate(EventOccurrenceRowTemplate {
                row,
                csrf_token: token,
            })
            .into_response()
        }
        Err(e) => partials::admin_alert(
            "error",
            &format!("Error overriding occurrence: {}", e),
            false,
        )
        .into_response(),
    }
}

/// POST — restore an exception. Returns the row's new "active" state.
pub async fn admin_restore_event_occurrence(
    State(event_admin_service): State<Arc<EventAdminService>>,
    State(event_repo): State<Arc<dyn EventRepository>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Path((series_id, idx)): Path<(String, i32)>,
) -> impl IntoResponse {
    let series_uuid = match uuid::Uuid::parse_str(&series_id) {
        Ok(u) => u,
        Err(_) => {
            return partials::admin_alert("error", "Invalid series ID", false).into_response()
        }
    };

    match event_admin_service
        .restore_event_occurrence(current_user.member.id, series_uuid, idx)
        .await
    {
        Ok((_, public_site)) if public_site.is_failed() => partials::admin_alert(
            "warning",
            &format!(
                "Exception restored. {}",
                public_site.admin_note().unwrap_or_default()
            ),
            true,
        )
        .into_response(),
        Ok((maybe_event, _)) => {
            // Whether the restore created a new row (cancelled →
            // re-materialize) or reset an existing one (overridden), the
            // current state on disk is the source of truth for the row.
            let event = match maybe_event {
                Some(e) => e,
                None => match event_repo.find_by_series_and_index(series_uuid, idx).await {
                    Ok(Some(e)) => e,
                    _ => {
                        return partials::admin_alert("success", "Exception restored", true)
                            .into_response();
                    }
                },
            };

            let token = csrf_service
                .generate_token(&session_info.session_id)
                .await
                .unwrap_or_default();
            let now = chrono::Utc::now();
            let row = OccurrenceRowInfo::from_active(&event, now);
            HtmlTemplate(EventOccurrenceRowTemplate {
                row,
                csrf_token: token,
            })
            .into_response()
        }
        Err(e) => partials::admin_alert(
            "error",
            &format!("Error restoring occurrence: {}", e),
            false,
        )
        .into_response(),
    }
}

// ---------------------------------------------------------------------
// Series detail page — extends the existing event detail with an
// occurrence list. The list lets the admin manage per-occurrence
// exceptions (cancel / override / restore) without leaving the page.

#[derive(Template)]
#[template(path = "admin/event_series_detail.html")]
pub struct AdminEventSeriesDetailTemplate {
    pub base: BaseContext,
    pub series_id: String,
    pub rows: Vec<OccurrenceRowInfo>,
    /// Pass pricing as the form wants it — dollars, blank for free, so
    /// the field renders empty rather than "0".
    pub member_price_input: String,
    pub guest_price_input: String,
    pub capacity_input: String,
    pub guest_registration_enabled: bool,
    /// True when the series has no end date, which is what makes a pass
    /// price illegal. The form says so instead of only rejecting on save.
    pub is_open_ended: bool,
    /// True when a pass costs a member money — drives the roster's
    /// money-state columns.
    pub is_paid_class: bool,
    /// Who bought a pass, with their payment state. Empty for a class
    /// nobody has enrolled in.
    pub enrollments: Vec<RosterRow>,
    /// The shareable public class-registration URL, or `None` when the
    /// class isn't publicly enrollable.
    pub registration_url: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub async fn admin_event_series_detail_page(
    State(settings): State<Arc<crate::config::Settings>>,
    State(event_repo): State<Arc<dyn EventRepository>>,
    State(event_series_repo): State<Arc<dyn EventSeriesRepository>>,
    State(enrollment_repo): State<Arc<dyn crate::repository::SeriesEnrollmentRepository>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Path(series_id): Path<String>,
) -> impl IntoResponse {
    let base = BaseContext::for_member(&csrf_service, &current_user, &session_info).await;
    let sid = match uuid::Uuid::parse_str(&series_id) {
        Ok(u) => u,
        Err(_) => {
            return partials::admin_alert("error", "Invalid series ID", false).into_response()
        }
    };

    // Fetch all events (we're going to filter to this series) and all
    // exceptions for the series. The events list is bounded by the
    // materialization horizon, so this is at most ~52 weekly rows +
    // outliers. Cheap to page through in Rust.
    let now = chrono::Utc::now();
    let all_events = event_repo.list(1000, 0).await.unwrap_or_default();
    let mut series_events: Vec<_> = all_events
        .into_iter()
        .filter(|e| e.series_id == Some(sid))
        .collect();
    series_events.sort_by_key(|e| e.occurrence_index.unwrap_or(0));

    let exceptions = event_series_repo
        .list_exceptions_for_series(sid)
        .await
        .unwrap_or_default();

    let mut rows: Vec<OccurrenceRowInfo> = Vec::new();
    for event in &series_events {
        let idx = event.occurrence_index.unwrap_or(0);
        let ex = exceptions.iter().find(|e| e.occurrence_index == idx);
        let mut row = OccurrenceRowInfo::from_active(event, now);
        if let Some(ex) = ex {
            row.state = match ex.kind {
                crate::domain::OccurrenceExceptionKind::Cancelled => "cancelled",
                crate::domain::OccurrenceExceptionKind::Overridden => "overridden",
            };
            row.reason = ex.audit_reason.clone();
        }
        rows.push(row);
    }
    // Tack on cancelled-only exceptions whose events row has been deleted.
    let present_indices: std::collections::HashSet<i32> =
        rows.iter().map(|r| r.occurrence_index).collect();
    for ex in &exceptions {
        if present_indices.contains(&ex.occurrence_index) {
            continue;
        }
        if matches!(ex.kind, crate::domain::OccurrenceExceptionKind::Cancelled) {
            rows.push(OccurrenceRowInfo {
                series_id: series_id.clone(),
                occurrence_index: ex.occurrence_index,
                event_id: None,
                title: String::new(),
                start_time: String::new(),
                location: None,
                is_past: false,
                state: "cancelled",
                reason: ex.audit_reason.clone(),
            });
        }
    }
    rows.sort_by_key(|r| r.occurrence_index);

    // The series row carries the pass pricing; a missing row means the
    // page is being asked about a series that no longer exists, which
    // renders as a free, empty class rather than a 500.
    let series = event_series_repo.find_by_id(sid).await.ok().flatten();
    let pricing = series
        .as_ref()
        .map(|s| s.pricing())
        .unwrap_or_else(Default::default);
    let is_open_ended = series.as_ref().is_none_or(|s| s.until_date.is_none());
    // One home for the enrollability rule (`EventSeries::publicly_enrollable`),
    // so this page can't disagree with the public one about whether a URL
    // exists.
    let registration_url = series
        .as_ref()
        .zip(series_events.first())
        .filter(|(s, occ)| s.publicly_enrollable(occ))
        .map(|(s, _)| {
            format!(
                "{}/classes/{}/register",
                settings.server.base_url.trim_end_matches('/'),
                s.id,
            )
        });

    let enrollments: Vec<RosterRow> = enrollment_repo
        .roster(sid)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(RosterRow::from_entry)
        .collect();

    HtmlTemplate(AdminEventSeriesDetailTemplate {
        base,
        series_id,
        rows,
        member_price_input: dollars_or_blank(pricing.member_price_cents),
        guest_price_input: dollars_or_blank(pricing.guest_price_cents),
        capacity_input: pricing
            .max_enrollments
            .map(|c| c.to_string())
            .unwrap_or_default(),
        guest_registration_enabled: pricing.guest_registration_enabled,
        is_open_ended,
        is_paid_class: pricing.is_paid_class(),
        enrollments,
        registration_url,
    })
    .into_response()
}

/// Dollars for a form field, or an empty string for a free class — a `0`
/// in a price box reads like a set price rather than an absent one.
fn dollars_or_blank(cents: i64) -> String {
    if cents > 0 {
        format!("{:.2}", cents as f64 / 100.0)
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A guest name with a double-quote in it must survive to the release
    /// control. It can't ride in `hx-vals`: the browser decodes the
    /// attribute's `&quot;` back to `"` before HTMX parses the JSON, so
    /// the vals are unparseable and the button does nothing. As a form
    /// value the same escaping is exactly right.
    #[test]
    fn release_control_survives_a_quoted_guest_name() {
        let row = RosterRow {
            member_id: String::new(),
            guest_name: r#"John "The Man" Doe"#.to_string(),
            guest_email: "john@example.com".to_string(),
            is_guest: true,
            name: r#"John "The Man" Doe"#.to_string(),
            email: "john@example.com".to_string(),
            status: "PendingPayment".to_string(),
            payment_state: "Awaiting payment".to_string(),
            amount_display: "$120.00".to_string(),
            is_pending_payment: true,
            refundable_payment_id: String::new(),
        };
        let html = AdminEventSeriesDetailTemplate {
            base: BaseContext::for_anon(),
            series_id: "s1".to_string(),
            rows: vec![],
            member_price_input: String::new(),
            guest_price_input: String::new(),
            capacity_input: String::new(),
            guest_registration_enabled: false,
            is_open_ended: false,
            is_paid_class: true,
            enrollments: vec![row],
            registration_url: None,
        }
        .render()
        .unwrap();

        assert!(
            !html.contains("hx-vals"),
            "identity must travel as form fields, not JSON in an attribute",
        );
        assert!(
            html.contains(
                r#"<input type="hidden" name="guest_name" value="John &quot;The Man&quot; Doe">"#
            ),
            "the quoted name should be attribute-escaped in a hidden input:\n{html}",
        );
    }
}
