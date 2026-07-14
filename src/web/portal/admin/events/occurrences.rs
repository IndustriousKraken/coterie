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
        Ok(()) => {
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
        Ok(event) => {
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
        Ok(maybe_event) => {
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
}

pub async fn admin_event_series_detail_page(
    State(event_repo): State<Arc<dyn EventRepository>>,
    State(event_series_repo): State<Arc<dyn EventSeriesRepository>>,
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

    HtmlTemplate(AdminEventSeriesDetailTemplate {
        base,
        series_id,
        rows,
    })
    .into_response()
}
