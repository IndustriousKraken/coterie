use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Multipart, Path, Query, State},
    response::IntoResponse,
    Extension,
};
use serde::Deserialize;

use crate::{
    api::{
        middleware::auth::{CurrentUser, SessionInfo},
        state::EventBasicTypeService,
    },
    auth::CsrfService,
    config::Settings,
    domain::OccurrenceOverride,
    repository::{EventRepository, EventSeriesRepository},
    service::event_admin_service::{CreateEventInput, EventAdminService, UpdateEventInput},
    web::portal::admin::partials,
    web::templates::{BaseContext, HtmlTemplate},
    web::uploads::save_uploaded_file,
};

/// Simple struct for type options in dropdowns
#[derive(Clone)]
pub struct TypeOption {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub color: Option<String>,
}

#[derive(Template)]
#[template(path = "admin/events.html")]
pub struct AdminEventsTemplate {
    pub base: BaseContext,
    pub events: Vec<AdminEventInfo>,
    pub total_events: i64,
    pub current_page: i64,
    pub per_page: i64,
    pub total_pages: i64,
    pub search_query: String,
    pub type_filter: String,
    pub visibility_filter: String,
    pub time_filter: String,
    pub sort_field: String,
    pub sort_order: String,
}

#[derive(Template)]
#[template(path = "admin/events_table.html")]
pub struct AdminEventsTableTemplate {
    pub events: Vec<AdminEventInfo>,
    pub total_events: i64,
    pub current_page: i64,
    pub per_page: i64,
    pub total_pages: i64,
    pub search_query: String,
    pub type_filter: String,
    pub visibility_filter: String,
    pub time_filter: String,
    pub sort_field: String,
    pub sort_order: String,
}

#[derive(Clone)]
pub struct AdminEventInfo {
    pub id: String,
    pub title: String,
    pub event_type: String,
    pub visibility: String,
    pub start_time: String,
    pub start_time_raw: chrono::DateTime<chrono::Utc>,
    pub end_time: Option<String>,
    pub location: Option<String>,
    pub image_url: Option<String>,
    pub attendee_count: i64,
    pub max_attendees: Option<i32>,
    pub rsvp_required: bool,
    pub is_past: bool,
}

#[derive(Debug, Deserialize)]
pub struct AdminEventsQuery {
    pub q: Option<String>,
    #[serde(rename = "type")]
    pub event_type: Option<String>,
    pub visibility: Option<String>,
    pub time: Option<String>,
    pub page: Option<i64>,
    pub sort: Option<String>,
    pub order: Option<String>,
}

pub async fn admin_events_page(
    State(event_repo): State<Arc<dyn EventRepository>>,
    State(csrf_service): State<Arc<CsrfService>>,
    headers: axum::http::HeaderMap,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Query(query): Query<AdminEventsQuery>,
) -> impl IntoResponse {
    let is_htmx = headers.get("HX-Request").is_some();

    let base = BaseContext::for_member(&csrf_service, &current_user, &session_info).await;

    let page = query.page.unwrap_or(1).max(1);
    let per_page: i64 = 20;
    let offset = (page - 1) * per_page;

    let search_query = query.q.clone().unwrap_or_default().to_lowercase();
    let type_filter = query.event_type.clone().unwrap_or_default();
    let visibility_filter = query.visibility.clone().unwrap_or_default();
    let time_filter = query.time.clone().unwrap_or_else(|| "upcoming".to_string());
    let sort_field = query
        .sort
        .clone()
        .unwrap_or_else(|| "start_time".to_string());
    let sort_order = query.order.clone().unwrap_or_else(|| "asc".to_string());

    let all_events = event_repo.list(1000, 0).await.unwrap_or_default();

    let now = chrono::Utc::now();

    let mut filtered_events: Vec<_> = all_events
        .into_iter()
        .filter(|e| {
            if !search_query.is_empty() {
                let matches = e.title.to_lowercase().contains(&search_query)
                    || e.description.to_lowercase().contains(&search_query)
                    || e.location
                        .as_ref()
                        .map(|l| l.to_lowercase().contains(&search_query))
                        .unwrap_or(false);
                if !matches {
                    return false;
                }
            }
            if !type_filter.is_empty() && format!("{:?}", e.event_type) != type_filter {
                return false;
            }
            if !visibility_filter.is_empty() && format!("{:?}", e.visibility) != visibility_filter {
                return false;
            }
            match time_filter.as_str() {
                "upcoming" => e.start_time > now,
                "past" => e.start_time <= now,
                _ => true,
            }
        })
        .collect();

    filtered_events.sort_by(|a, b| {
        let cmp = match sort_field.as_str() {
            "title" => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
            "type" => format!("{:?}", a.event_type).cmp(&format!("{:?}", b.event_type)),
            "visibility" => format!("{:?}", a.visibility).cmp(&format!("{:?}", b.visibility)),
            "start_time" | _ => a.start_time.cmp(&b.start_time),
        };
        if sort_order == "desc" {
            cmp.reverse()
        } else {
            cmp
        }
    });

    let total_events = filtered_events.len() as i64;
    let total_pages = (total_events + per_page - 1) / per_page;

    let mut paginated_events = Vec::new();
    for e in filtered_events
        .into_iter()
        .skip(offset as usize)
        .take(per_page as usize)
    {
        let attendee_count = event_repo.get_attendee_count(e.id).await.unwrap_or(0);

        paginated_events.push(AdminEventInfo {
            id: e.id.to_string(),
            title: e.title,
            event_type: format!("{:?}", e.event_type),
            visibility: format!("{:?}", e.visibility),
            start_time: e.start_time.format("%b %d, %Y %H:%M").to_string(),
            start_time_raw: e.start_time,
            end_time: e.end_time.map(|t| t.format("%H:%M").to_string()),
            location: e.location,
            image_url: e.image_url,
            attendee_count,
            max_attendees: e.max_attendees,
            rsvp_required: e.rsvp_required,
            is_past: e.start_time <= now,
        });
    }

    let search_query_val = query.q.unwrap_or_default();
    let type_filter_val = query.event_type.unwrap_or_default();
    let visibility_filter_val = query.visibility.unwrap_or_default();

    if is_htmx {
        HtmlTemplate(AdminEventsTableTemplate {
            events: paginated_events,
            total_events,
            current_page: page,
            per_page,
            total_pages,
            search_query: search_query_val,
            type_filter: type_filter_val,
            visibility_filter: visibility_filter_val,
            time_filter,
            sort_field,
            sort_order,
        })
        .into_response()
    } else {
        HtmlTemplate(AdminEventsTemplate {
            base,
            events: paginated_events,
            total_events,
            current_page: page,
            per_page,
            total_pages,
            search_query: search_query_val,
            type_filter: type_filter_val,
            visibility_filter: visibility_filter_val,
            time_filter,
            sort_field,
            sort_order,
        })
        .into_response()
    }
}

#[derive(Template)]
#[template(path = "admin/event_detail.html")]
pub struct AdminEventDetailTemplate {
    pub base: BaseContext,
    pub event: AdminEventDetail,
    pub event_types: Vec<TypeOption>,
}

pub struct AdminEventDetail {
    pub id: String,
    pub title: String,
    pub description: String,
    pub event_type: String,
    pub visibility: String,
    pub start_time: String,
    pub start_time_input: String,
    pub end_time: Option<String>,
    pub end_time_input: Option<String>,
    pub location: Option<String>,
    pub max_attendees: Option<i32>,
    pub rsvp_required: bool,
    pub image_url: Option<String>,
    pub attendee_count: i64,
    pub is_past: bool,
    pub created_at: String,
    pub updated_at: String,
    /// True when this event is one occurrence of a recurring series.
    /// Drives the "edit this / edit this and future" radio + the
    /// "cancel just this / end the series / delete entire series"
    /// dropdown on the detail page.
    pub is_series: bool,
    pub occurrence_index: Option<i32>,
    pub series_id: Option<String>,
}

pub async fn admin_event_detail_page(
    State(event_repo): State<Arc<dyn EventRepository>>,
    State(event_type_service): State<EventBasicTypeService>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Path(event_id): Path<String>,
) -> impl IntoResponse {
    let base = BaseContext::for_member(&csrf_service, &current_user, &session_info).await;

    let id = match uuid::Uuid::parse_str(&event_id) {
        Ok(id) => id,
        Err(_) => return partials::admin_alert("error", "Invalid event ID", false).into_response(),
    };

    let event = match event_repo.find_by_id(id).await {
        Ok(Some(e)) => e,
        Ok(None) => {
            return partials::admin_alert("error", "Event not found", false).into_response()
        }
        Err(_) => {
            return partials::admin_alert("error", "Error loading event", false).into_response()
        }
    };

    let attendee_count = event_repo.get_attendee_count(event.id).await.unwrap_or(0);

    let now = chrono::Utc::now();

    let detail = AdminEventDetail {
        id: event.id.to_string(),
        title: event.title,
        description: event.description,
        event_type: format!("{:?}", event.event_type),
        visibility: format!("{:?}", event.visibility),
        start_time: event.start_time.format("%b %d, %Y %H:%M").to_string(),
        start_time_input: event.start_time.format("%Y-%m-%dT%H:%M").to_string(),
        end_time: event
            .end_time
            .map(|t| t.format("%b %d, %Y %H:%M").to_string()),
        end_time_input: event
            .end_time
            .map(|t| t.format("%Y-%m-%dT%H:%M").to_string()),
        location: event.location,
        max_attendees: event.max_attendees,
        rsvp_required: event.rsvp_required,
        image_url: event.image_url,
        attendee_count,
        is_past: event.start_time <= now,
        created_at: event.created_at.format("%b %d, %Y %H:%M").to_string(),
        updated_at: event.updated_at.format("%b %d, %Y %H:%M").to_string(),
        is_series: event.series_id.is_some(),
        occurrence_index: event.occurrence_index,
        series_id: event.series_id.map(|id| id.to_string()),
    };

    // Fetch active event types for the dropdown
    let event_types = event_type_service
        .0
        .list(false)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|t| TypeOption {
            id: t.id.to_string(),
            name: t.name,
            slug: t.slug,
            color: t.color,
        })
        .collect();

    HtmlTemplate(AdminEventDetailTemplate {
        base,
        event: detail,
        event_types,
    })
    .into_response()
}

#[derive(Template)]
#[template(path = "admin/event_new.html")]
pub struct AdminNewEventTemplate {
    pub base: BaseContext,
    pub event_types: Vec<TypeOption>,
}

pub async fn admin_new_event_page(
    State(event_type_service): State<EventBasicTypeService>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> impl IntoResponse {
    let base = BaseContext::for_member(&csrf_service, &current_user, &session_info).await;

    // Fetch active event types for the dropdown
    let event_types = event_type_service
        .0
        .list(false)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|t| TypeOption {
            id: t.id.to_string(),
            name: t.name,
            slug: t.slug,
            color: t.color,
        })
        .collect();

    HtmlTemplate(AdminNewEventTemplate { base, event_types }).into_response()
}

pub async fn admin_create_event(
    State(settings): State<Arc<Settings>>,
    State(event_admin_service): State<Arc<EventAdminService>>,
    Extension(current_user): Extension<CurrentUser>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    use crate::domain::{EventType, EventVisibility};

    // Parse multipart form
    let mut title = String::new();
    let mut description = String::new();
    let mut event_type_str = String::new();
    let mut visibility_str = String::new();
    let mut start_time_str = String::new();
    let mut end_time_str = String::new();
    let mut location_str = String::new();
    let mut max_attendees: Option<i32> = None;
    let mut rsvp_required = false;
    let mut image_url: Option<String> = None;
    // Recurrence form fields. `repeat_kind` defaults to "none" so an
    // unchecked form behaves identically to the pre-recurrence flow.
    let mut repeat_kind = String::from("none");
    let mut repeat_interval: u32 = 1;
    let mut repeat_weekdays: Vec<String> = Vec::new();
    let mut repeat_day: Option<u32> = None;
    let mut repeat_weekday = String::from("mon");
    let mut repeat_ordinal: i32 = 1;
    let mut repeat_until_str = String::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();

        match name.as_str() {
            "csrf_token" => {
                let _ = field.text().await;
            }
            "title" => title = field.text().await.unwrap_or_default(),
            "description" => description = field.text().await.unwrap_or_default(),
            "event_type" => event_type_str = field.text().await.unwrap_or_default(),
            "visibility" => visibility_str = field.text().await.unwrap_or_default(),
            "start_time" => start_time_str = field.text().await.unwrap_or_default(),
            "end_time" => end_time_str = field.text().await.unwrap_or_default(),
            "location" => location_str = field.text().await.unwrap_or_default(),
            "max_attendees" => {
                if let Ok(text) = field.text().await {
                    max_attendees = text.parse().ok();
                }
            }
            "rsvp_required" => {
                rsvp_required = true;
                let _ = field.text().await;
            }
            "repeat_kind" => repeat_kind = field.text().await.unwrap_or_default(),
            "repeat_interval" => {
                if let Ok(text) = field.text().await {
                    if let Ok(n) = text.parse() {
                        repeat_interval = n;
                    }
                }
            }
            "repeat_weekdays" => {
                // Multipart sends one field per checked box; collect them.
                if let Ok(text) = field.text().await {
                    repeat_weekdays.push(text);
                }
            }
            "repeat_day" => {
                if let Ok(text) = field.text().await {
                    repeat_day = text.parse().ok();
                }
            }
            "repeat_weekday" => repeat_weekday = field.text().await.unwrap_or_default(),
            "repeat_ordinal" => {
                if let Ok(text) = field.text().await {
                    if let Ok(n) = text.parse() {
                        repeat_ordinal = n;
                    }
                }
            }
            "repeat_until" => repeat_until_str = field.text().await.unwrap_or_default(),
            "image" => {
                let filename = field.file_name().unwrap_or("").to_string();
                if !filename.is_empty() {
                    if let Ok(data) = field.bytes().await {
                        if !data.is_empty() {
                            match save_uploaded_file(
                                &settings.server.uploads_path(),
                                &filename,
                                &data,
                            )
                            .await
                            {
                                Ok(path) => image_url = Some(path),
                                Err(e) => {
                                    return partials::admin_alert(
                                        "error",
                                        &format!("Error uploading image: {}", e),
                                        false,
                                    )
                                    .into_response()
                                }
                            }
                        }
                    }
                }
            }
            _ => {
                let _ = field.bytes().await;
            }
        }
    }

    let event_type = match event_type_str.as_str() {
        "Meeting" => EventType::Meeting,
        "Workshop" => EventType::Workshop,
        "CTF" => EventType::CTF,
        "Social" => EventType::Social,
        "Training" => EventType::Training,
        _ => EventType::Meeting,
    };

    let visibility = match visibility_str.as_str() {
        "Public" => EventVisibility::Public,
        "MembersOnly" => EventVisibility::MembersOnly,
        "AdminOnly" => EventVisibility::AdminOnly,
        _ => EventVisibility::MembersOnly,
    };

    let start_time = match chrono::NaiveDateTime::parse_from_str(&start_time_str, "%Y-%m-%dT%H:%M")
    {
        Ok(dt) => chrono::DateTime::from_naive_utc_and_offset(dt, chrono::Utc),
        Err(_) => {
            return partials::admin_alert("error", "Invalid start time", false).into_response()
        }
    };

    let end_time = if end_time_str.is_empty() {
        None
    } else {
        chrono::NaiveDateTime::parse_from_str(&end_time_str, "%Y-%m-%dT%H:%M")
            .ok()
            .map(|dt| chrono::DateTime::from_naive_utc_and_offset(dt, chrono::Utc))
    };

    // Build the recurrence rule, if the admin asked for one. The
    // service decides series-vs-single by inspecting input.recurrence.
    let recurrence = if repeat_kind != "none" && !repeat_kind.is_empty() {
        match build_recurrence(
            &repeat_kind,
            repeat_interval,
            &repeat_weekdays,
            repeat_day,
            &repeat_weekday,
            repeat_ordinal,
        ) {
            Ok(r) => Some(r),
            Err(msg) => {
                return partials::admin_alert(
                    "error",
                    &format!("Invalid recurrence: {}", msg),
                    false,
                )
                .into_response()
            }
        }
    } else {
        None
    };
    let recurrence_until = if recurrence.is_some() {
        parse_until(&repeat_until_str)
    } else {
        None
    };

    let input = CreateEventInput {
        title,
        description,
        event_type,
        event_type_id: None,
        visibility,
        start_time,
        end_time,
        location: if location_str.is_empty() {
            None
        } else {
            Some(location_str)
        },
        max_attendees,
        rsvp_required,
        image_url,
        recurrence,
        recurrence_until,
    };

    match event_admin_service
        .create(current_user.member.id, input)
        .await
    {
        Ok(created) => {
            axum::response::Redirect::to(&format!("/portal/admin/events/{}", created.id))
                .into_response()
        }
        Err(e) => partials::admin_alert("error", &format!("Error creating event: {}", e), false)
            .into_response(),
    }
}

/// Build a `Recurrence` from form fields. The error returned is the
/// human-readable message we render back to the admin form.
fn build_recurrence(
    kind: &str,
    interval: u32,
    weekdays: &[String],
    day: Option<u32>,
    weekday: &str,
    ordinal: i32,
) -> std::result::Result<crate::domain::Recurrence, &'static str> {
    use crate::domain::{Recurrence, WeekdayCode};

    fn parse_wd(s: &str) -> std::result::Result<WeekdayCode, &'static str> {
        match s {
            "mon" => Ok(WeekdayCode::Mon),
            "tue" => Ok(WeekdayCode::Tue),
            "wed" => Ok(WeekdayCode::Wed),
            "thu" => Ok(WeekdayCode::Thu),
            "fri" => Ok(WeekdayCode::Fri),
            "sat" => Ok(WeekdayCode::Sat),
            "sun" => Ok(WeekdayCode::Sun),
            _ => Err("invalid weekday"),
        }
    }

    let rule = match kind {
        "weekly" => {
            let parsed = weekdays
                .iter()
                .map(|s| parse_wd(s))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Recurrence::WeeklyByDay {
                interval,
                weekdays: parsed,
            }
        }
        "monthly_dom" => {
            let day = day.ok_or("day-of-month is required")?;
            Recurrence::MonthlyByDayOfMonth { interval, day }
        }
        "monthly_weekday" => {
            let weekday = parse_wd(weekday)?;
            Recurrence::MonthlyByWeekdayOrdinal {
                interval,
                weekday,
                ordinal,
            }
        }
        _ => return Err("unknown recurrence kind"),
    };
    rule.validate()?;
    Ok(rule)
}

fn parse_until(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    if s.is_empty() {
        return None;
    }
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M")
        .ok()
        .map(|dt| chrono::DateTime::from_naive_utc_and_offset(dt, chrono::Utc))
}

pub async fn admin_update_event(
    State(settings): State<Arc<Settings>>,
    State(event_repo): State<Arc<dyn EventRepository>>,
    State(event_admin_service): State<Arc<EventAdminService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(event_id): Path<String>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    use crate::domain::{EventType, EventVisibility};

    let id = match uuid::Uuid::parse_str(&event_id) {
        Ok(id) => id,
        Err(_) => return partials::admin_alert("error", "Invalid event ID", false).into_response(),
    };

    let existing = match event_repo.find_by_id(id).await {
        Ok(Some(e)) => e,
        Ok(None) => {
            return partials::admin_alert("error", "Event not found", false).into_response()
        }
        Err(_) => {
            return partials::admin_alert("error", "Error loading event", false).into_response()
        }
    };

    // Parse multipart form
    let mut title = String::new();
    let mut description = String::new();
    let mut event_type_str = String::new();
    let mut visibility_str = String::new();
    let mut start_time_str = String::new();
    let mut end_time_str = String::new();
    let mut location_str = String::new();
    let mut max_attendees: Option<i32> = None;
    let mut rsvp_required = false;
    let mut new_image_url: Option<String> = None;
    let mut remove_image = false;
    // For series occurrences: "this" (default), "this_and_future".
    // Ignored for one-off events.
    let mut edit_scope = String::from("this");

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();

        match name.as_str() {
            "csrf_token" => {
                let _ = field.text().await;
            }
            "title" => title = field.text().await.unwrap_or_default(),
            "description" => description = field.text().await.unwrap_or_default(),
            "event_type" => event_type_str = field.text().await.unwrap_or_default(),
            "visibility" => visibility_str = field.text().await.unwrap_or_default(),
            "start_time" => start_time_str = field.text().await.unwrap_or_default(),
            "end_time" => end_time_str = field.text().await.unwrap_or_default(),
            "location" => location_str = field.text().await.unwrap_or_default(),
            "max_attendees" => {
                if let Ok(text) = field.text().await {
                    max_attendees = text.parse().ok();
                }
            }
            "rsvp_required" => {
                rsvp_required = true;
                let _ = field.text().await;
            }
            "edit_scope" => edit_scope = field.text().await.unwrap_or_default(),
            "remove_image" => {
                remove_image = true;
                let _ = field.text().await;
            }
            "image" => {
                let filename = field.file_name().unwrap_or("").to_string();
                if !filename.is_empty() {
                    if let Ok(data) = field.bytes().await {
                        if !data.is_empty() {
                            match save_uploaded_file(
                                &settings.server.uploads_path(),
                                &filename,
                                &data,
                            )
                            .await
                            {
                                Ok(path) => new_image_url = Some(path),
                                Err(e) => {
                                    return partials::admin_alert(
                                        "error",
                                        &format!("Error uploading image: {}", e),
                                        false,
                                    )
                                    .into_response()
                                }
                            }
                        }
                    }
                }
            }
            _ => {
                let _ = field.bytes().await;
            }
        }
    }

    let event_type = match event_type_str.as_str() {
        "Meeting" => EventType::Meeting,
        "Workshop" => EventType::Workshop,
        "CTF" => EventType::CTF,
        "Social" => EventType::Social,
        "Training" => EventType::Training,
        _ => EventType::Meeting,
    };

    let visibility = match visibility_str.as_str() {
        "Public" => EventVisibility::Public,
        "MembersOnly" => EventVisibility::MembersOnly,
        "AdminOnly" => EventVisibility::AdminOnly,
        _ => EventVisibility::MembersOnly,
    };

    let start_time = match chrono::NaiveDateTime::parse_from_str(&start_time_str, "%Y-%m-%dT%H:%M") {
        Ok(dt) => chrono::DateTime::from_naive_utc_and_offset(dt, chrono::Utc),
        Err(_) => return axum::response::Html(r#"<div class="px-4 py-3 bg-red-100 text-red-800 rounded-md text-sm">Invalid start time</div>"#.to_string()).into_response(),
    };

    let end_time = if end_time_str.is_empty() {
        None
    } else {
        chrono::NaiveDateTime::parse_from_str(&end_time_str, "%Y-%m-%dT%H:%M")
            .ok()
            .map(|dt| chrono::DateTime::from_naive_utc_and_offset(dt, chrono::Utc))
    };

    // Determine final image_url: new upload > remove > keep existing.
    // Also capture what (if anything) we need to delete from disk.
    let old_image = existing.image_url.clone();
    let image_url = if new_image_url.is_some() {
        new_image_url
    } else if remove_image {
        None
    } else {
        old_image.clone()
    };
    // Old file should be dropped when we either replaced it or removed it.
    let image_to_delete = if image_url != old_image {
        old_image
    } else {
        None
    };

    let input = UpdateEventInput {
        title,
        description,
        event_type,
        event_type_id: existing.event_type_id,
        visibility,
        start_time,
        end_time,
        location: if location_str.is_empty() {
            None
        } else {
            Some(location_str)
        },
        max_attendees,
        rsvp_required,
        image_url,
    };

    // Always update THIS row first — the radio defaults to "this" and
    // even the "this and future" path expects this row to reflect the
    // form values too.
    let updated = match event_admin_service
        .update_one(current_user.member.id, id, input.clone())
        .await
    {
        Ok(u) => u,
        Err(e) => {
            return partials::admin_alert("error", &format!("Error updating event: {}", e), false)
                .into_response();
        }
    };
    crate::web::uploads::delete_if_upload(
        &settings.server.uploads_path(),
        image_to_delete.as_deref(),
    )
    .await;

    // Series-aware "edit this and all future" path: apply the same
    // mutable subset to every later occurrence in the series.
    let mut future_count = 0u64;
    if edit_scope == "this_and_future" {
        if let Some(series_id) = existing.series_id {
            match event_admin_service
                .update_series_from(current_user.member.id, series_id, updated.start_time, input)
                .await
            {
                Ok(n) => future_count = n,
                Err(e) => tracing::error!("edit-this-and-future failed for event {}: {}", id, e,),
            }
        }
    }

    let msg = if edit_scope == "this_and_future" {
        format!(
            "Event updated. {} future occurrences also updated.",
            future_count.saturating_sub(1)
        )
    } else {
        "Event updated successfully".to_string()
    };
    partials::admin_alert("success", &msg, false).into_response()
}

pub async fn admin_delete_event(
    State(settings): State<Arc<Settings>>,
    State(event_repo): State<Arc<dyn EventRepository>>,
    State(event_admin_service): State<Arc<EventAdminService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(event_id): Path<String>,
    axum::Form(form): axum::Form<DeleteEventForm>,
) -> impl IntoResponse {
    let id = match uuid::Uuid::parse_str(&event_id) {
        Ok(id) => id,
        Err(_) => return partials::admin_alert("error", "Invalid event ID", false).into_response(),
    };

    let event = match event_repo.find_by_id(id).await {
        Ok(Some(e)) => e,
        Ok(None) => {
            return partials::admin_alert("error", "Event not found", false).into_response()
        }
        Err(_) => {
            return partials::admin_alert("error", "Error loading event", false).into_response()
        }
    };

    // Series-aware delete scope. "this" is the default and behaves
    // like the pre-recurrence flow (drop one row). The other two
    // require the event to actually be in a series — if not, fall
    // through silently to "this" so a misclick can't 500.
    let scope = form.scope.as_deref().unwrap_or("this");
    let series_id = event.series_id;

    if (scope == "end_series" || scope == "delete_series") && series_id.is_some() {
        let sid = series_id.unwrap();

        if scope == "end_series" {
            match event_admin_service
                .end_series(current_user.member.id, sid, event.start_time)
                .await
            {
                Ok(_) => {
                    return axum::response::Redirect::to(&format!("/portal/admin/events/{}", id))
                        .into_response();
                }
                Err(e) => {
                    return partials::admin_alert(
                        "error",
                        &format!("Error ending series: {}", e),
                        false,
                    )
                    .into_response();
                }
            }
        }

        match event_admin_service
            .delete_series(current_user.member.id, sid)
            .await
        {
            Ok(_) => {
                return axum::response::Redirect::to("/portal/admin/events").into_response();
            }
            Err(e) => {
                return partials::admin_alert(
                    "error",
                    &format!("Error deleting series: {}", e),
                    false,
                )
                .into_response();
            }
        }
    }

    // Default: delete this single row, scope=="this".
    let image_to_delete = event.image_url.clone();
    match event_admin_service
        .delete_one(current_user.member.id, id)
        .await
    {
        Ok(_) => {
            crate::web::uploads::delete_if_upload(
                &settings.server.uploads_path(),
                image_to_delete.as_deref(),
            )
            .await;
            axum::response::Redirect::to("/portal/admin/events").into_response()
        }
        Err(e) => partials::admin_alert("error", &format!("Error deleting event: {}", e), false)
            .into_response(),
    }
}

#[derive(serde::Deserialize, Default)]
pub struct DeleteEventForm {
    /// One of "this" (default), "end_series", "delete_series". The
    /// last two are no-ops when the event isn't in a series.
    pub scope: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub csrf_token: String,
}

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
            is_past: event.start_time <= now,
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
