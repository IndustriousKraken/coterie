use askama::Template;
use axum::{
    extract::{State, Query, Path, Multipart},
    response::IntoResponse,
    Extension,
};
use serde::Deserialize;

use crate::{
    api::{
        middleware::auth::{CurrentUser, SessionInfo},
        state::AppState,
    },
    web::templates::{HtmlTemplate, UserInfo},
    web::uploads::save_uploaded_file,
};
use crate::web::portal::is_admin;

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
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub csrf_token: String,
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
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Query(query): Query<AdminEventsQuery>,
) -> impl IntoResponse {
    let is_htmx = headers.get("HX-Request").is_some();

    if !is_admin(&current_user.member) {
        return HtmlTemplate(AdminEventsTemplate {
            current_user: None,
            is_admin: false,
            csrf_token: String::new(),
            events: vec![],
            total_events: 0,
            current_page: 1,
            per_page: 20,
            total_pages: 0,
            search_query: String::new(),
            type_filter: String::new(),
            visibility_filter: String::new(),
            time_filter: "upcoming".to_string(),
            sort_field: "start_time".to_string(),
            sort_order: "asc".to_string(),
        }).into_response();
    }

    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let csrf_token = state.service_context.csrf_service
        .generate_token(&session_info.session_id)
        .await
        .unwrap_or_else(|_| "error".to_string());

    let page = query.page.unwrap_or(1).max(1);
    let per_page: i64 = 20;
    let offset = (page - 1) * per_page;

    let search_query = query.q.clone().unwrap_or_default().to_lowercase();
    let type_filter = query.event_type.clone().unwrap_or_default();
    let visibility_filter = query.visibility.clone().unwrap_or_default();
    let time_filter = query.time.clone().unwrap_or_else(|| "upcoming".to_string());
    let sort_field = query.sort.clone().unwrap_or_else(|| "start_time".to_string());
    let sort_order = query.order.clone().unwrap_or_else(|| "asc".to_string());

    let all_events = state.service_context.event_repo
        .list(1000, 0)
        .await
        .unwrap_or_default();

    let now = chrono::Utc::now();

    let mut filtered_events: Vec<_> = all_events.into_iter()
        .filter(|e| {
            if !search_query.is_empty() {
                let matches = e.title.to_lowercase().contains(&search_query)
                    || e.description.to_lowercase().contains(&search_query)
                    || e.location.as_ref().map(|l| l.to_lowercase().contains(&search_query)).unwrap_or(false);
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
        if sort_order == "desc" { cmp.reverse() } else { cmp }
    });

    let total_events = filtered_events.len() as i64;
    let total_pages = (total_events + per_page - 1) / per_page;

    let mut paginated_events = Vec::new();
    for e in filtered_events.into_iter().skip(offset as usize).take(per_page as usize) {
        let attendee_count = state.service_context.event_repo
            .get_attendee_count(e.id)
            .await
            .unwrap_or(0);

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
        }).into_response()
    } else {
        HtmlTemplate(AdminEventsTemplate {
            current_user: Some(user_info),
            is_admin: true,
            csrf_token,
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
        }).into_response()
    }
}

#[derive(Template)]
#[template(path = "admin/event_detail.html")]
pub struct AdminEventDetailTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub csrf_token: String,
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
}

pub async fn admin_event_detail_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Path(event_id): Path<String>,
) -> impl IntoResponse {
    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let csrf_token = state.service_context.csrf_service
        .generate_token(&session_info.session_id)
        .await
        .unwrap_or_else(|_| "error".to_string());

    let id = match uuid::Uuid::parse_str(&event_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html("Invalid event ID".to_string()).into_response(),
    };

    let event = match state.service_context.event_repo.find_by_id(id).await {
        Ok(Some(e)) => e,
        Ok(None) => return axum::response::Html("Event not found".to_string()).into_response(),
        Err(_) => return axum::response::Html("Error loading event".to_string()).into_response(),
    };

    let attendee_count = state.service_context.event_repo
        .get_attendee_count(event.id)
        .await
        .unwrap_or(0);

    let now = chrono::Utc::now();

    let detail = AdminEventDetail {
        id: event.id.to_string(),
        title: event.title,
        description: event.description,
        event_type: format!("{:?}", event.event_type),
        visibility: format!("{:?}", event.visibility),
        start_time: event.start_time.format("%b %d, %Y %H:%M").to_string(),
        start_time_input: event.start_time.format("%Y-%m-%dT%H:%M").to_string(),
        end_time: event.end_time.map(|t| t.format("%b %d, %Y %H:%M").to_string()),
        end_time_input: event.end_time.map(|t| t.format("%Y-%m-%dT%H:%M").to_string()),
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
    };

    // Fetch active event types for the dropdown
    let event_types = state.service_context.event_type_service
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
        current_user: Some(user_info),
        is_admin: true,
        csrf_token,
        event: detail,
        event_types,
    }).into_response()
}

#[derive(Template)]
#[template(path = "admin/event_new.html")]
pub struct AdminNewEventTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub csrf_token: String,
    pub event_types: Vec<TypeOption>,
}

pub async fn admin_new_event_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> impl IntoResponse {
    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let csrf_token = state.service_context.csrf_service
        .generate_token(&session_info.session_id)
        .await
        .unwrap_or_else(|_| "error".to_string());

    // Fetch active event types for the dropdown
    let event_types = state.service_context.event_type_service
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

    HtmlTemplate(AdminNewEventTemplate {
        current_user: Some(user_info),
        is_admin: true,
        csrf_token,
        event_types,
    }).into_response()
}

pub async fn admin_create_event(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    use crate::domain::{Event, EventType, EventVisibility};

    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

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
            "csrf_token" => { let _ = field.text().await; }
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
                    if let Ok(n) = text.parse() { repeat_interval = n; }
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
                    if let Ok(n) = text.parse() { repeat_ordinal = n; }
                }
            }
            "repeat_until" => repeat_until_str = field.text().await.unwrap_or_default(),
            "image" => {
                let filename = field.file_name().unwrap_or("").to_string();
                if !filename.is_empty() {
                    if let Ok(data) = field.bytes().await {
                        if !data.is_empty() {
                            match save_uploaded_file(&state.settings.server.uploads_path(), &filename, &data).await {
                                Ok(path) => image_url = Some(path),
                                Err(e) => return axum::response::Html(format!("Error uploading image: {}", crate::web::escape_html(&e.to_string()))).into_response(),
                            }
                        }
                    }
                }
            }
            _ => { let _ = field.bytes().await; }
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
        Err(_) => return axum::response::Html("Invalid start time".to_string()).into_response(),
    };

    let end_time = if end_time_str.is_empty() {
        None
    } else {
        chrono::NaiveDateTime::parse_from_str(&end_time_str, "%Y-%m-%dT%H:%M")
            .ok()
            .map(|dt| chrono::DateTime::from_naive_utc_and_offset(dt, chrono::Utc))
    };

    let event = Event {
        id: uuid::Uuid::new_v4(),
        title,
        description,
        event_type,
        event_type_id: None,
        visibility,
        start_time,
        end_time,
        location: if location_str.is_empty() { None } else { Some(location_str) },
        max_attendees,
        rsvp_required,
        image_url,
        created_by: current_user.member.id,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        series_id: None,
        occurrence_index: None,
    };

    // If the admin asked for a repeating series, route through the
    // RecurringEventService so the series row + 12 months of
    // occurrences materialize in one shot. Otherwise fall through to
    // the original single-event create path.
    if repeat_kind != "none" && !repeat_kind.is_empty() {
        let rule = match build_recurrence(
            &repeat_kind, repeat_interval, &repeat_weekdays,
            repeat_day, &repeat_weekday, repeat_ordinal,
        ) {
            Ok(r) => r,
            Err(msg) => return axum::response::Html(format!(
                "Invalid recurrence: {}", crate::web::escape_html(msg),
            )).into_response(),
        };
        let until = parse_until(&repeat_until_str);

        match state.service_context.recurring_event_service
            .create_series_with_initial_materialization(
                rule, event, until, current_user.member.id,
            ).await
        {
            Ok(created) => {
                let first = &created.occurrences[0];
                state.service_context.audit_service.log(
                    Some(current_user.member.id),
                    "create_event_series",
                    "event_series",
                    &created.series.id.to_string(),
                    None,
                    Some(&first.title),
                    None,
                ).await;
                // Single Discord notification for the entire series —
                // we treat it like one announcement, not 52.
                state.service_context.integration_manager
                    .handle_event(crate::integrations::IntegrationEvent::EventPublished(first.clone()))
                    .await;
                return axum::response::Redirect::to(
                    &format!("/portal/admin/events/{}", first.id),
                ).into_response();
            }
            Err(e) => return axum::response::Html(format!(
                "Error creating recurring event: {}",
                crate::web::escape_html(&e.to_string()),
            )).into_response(),
        }
    }

    match state.service_context.event_repo.create(event).await {
        Ok(created) => {
            state.service_context.audit_service.log(
                Some(current_user.member.id),
                "create_event",
                "event",
                &created.id.to_string(),
                None,
                Some(&created.title),
                None,
            ).await;
            // Notify integrations (Discord channel post if configured).
            // Routing by visibility happens inside DiscordIntegration.
            state.service_context.integration_manager
                .handle_event(crate::integrations::IntegrationEvent::EventPublished(created.clone()))
                .await;
            axum::response::Redirect::to(&format!("/portal/admin/events/{}", created.id)).into_response()
        }
        Err(e) => axum::response::Html(format!("Error creating event: {}", crate::web::escape_html(&e.to_string()))).into_response(),
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
            let parsed = weekdays.iter()
                .map(|s| parse_wd(s))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Recurrence::WeeklyByDay { interval, weekdays: parsed }
        }
        "monthly_dom" => {
            let day = day.ok_or("day-of-month is required")?;
            Recurrence::MonthlyByDayOfMonth { interval, day }
        }
        "monthly_weekday" => {
            let weekday = parse_wd(weekday)?;
            Recurrence::MonthlyByWeekdayOrdinal { interval, weekday, ordinal }
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
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Path(event_id): Path<String>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    use crate::domain::{Event, EventType, EventVisibility};

    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

    let id = match uuid::Uuid::parse_str(&event_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html("Invalid event ID".to_string()).into_response(),
    };

    let existing = match state.service_context.event_repo.find_by_id(id).await {
        Ok(Some(e)) => e,
        Ok(None) => return axum::response::Html("Event not found".to_string()).into_response(),
        Err(_) => return axum::response::Html("Error loading event".to_string()).into_response(),
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
            "csrf_token" => { let _ = field.text().await; }
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
                            match save_uploaded_file(&state.settings.server.uploads_path(), &filename, &data).await {
                                Ok(path) => new_image_url = Some(path),
                                Err(e) => return axum::response::Html(format!(r#"<div class="px-4 py-3 bg-red-100 text-red-800 rounded-md text-sm">Error uploading image: {}</div>"#, crate::web::escape_html(&e.to_string()))).into_response(),
                            }
                        }
                    }
                }
            }
            _ => { let _ = field.bytes().await; }
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
    let image_to_delete = if image_url != old_image { old_image } else { None };

    let updated_event = Event {
        id,
        title,
        description,
        event_type,
        event_type_id: existing.event_type_id,
        visibility,
        start_time,
        end_time,
        location: if location_str.is_empty() { None } else { Some(location_str) },
        max_attendees,
        rsvp_required,
        image_url,
        created_by: existing.created_by,
        created_at: existing.created_at,
        updated_at: chrono::Utc::now(),
        series_id: existing.series_id,
        occurrence_index: existing.occurrence_index,
    };

    // Always update THIS row first — the radio defaults to "this" and
    // even the "this and future" path expects this row to reflect the
    // form values too.
    let updated = match state.service_context.event_repo.update(id, updated_event.clone()).await {
        Ok(u) => u,
        Err(e) => {
            return axum::response::Html(format!(
                r#"<div class="px-4 py-3 bg-red-100 text-red-800 rounded-md text-sm">Error updating event: {}</div>"#,
                crate::web::escape_html(&e.to_string()),
            )).into_response();
        }
    };
    crate::web::uploads::delete_if_upload(
        &state.settings.server.uploads_path(),
        image_to_delete.as_deref(),
    ).await;

    // Series-aware "edit this and all future" path: apply the same
    // mutable subset to every later occurrence in the series.
    let mut future_count = 0u64;
    if edit_scope == "this_and_future" {
        if let Some(series_id) = updated_event.series_id {
            match state.service_context.event_repo
                .update_series_occurrences_from(
                    series_id, updated_event.start_time, &updated_event,
                ).await
            {
                Ok(n) => future_count = n,
                Err(e) => tracing::error!(
                    "edit-this-and-future failed for event {}: {}", id, e,
                ),
            }
        }
    }

    state.service_context.audit_service.log(
        Some(current_user.member.id),
        if edit_scope == "this_and_future" { "update_event_series_future" } else { "update_event" },
        "event",
        &id.to_string(),
        None,
        Some(&updated.title),
        None,
    ).await;

    let msg = if edit_scope == "this_and_future" {
        format!("Event updated. {} future occurrences also updated.", future_count.saturating_sub(1))
    } else {
        "Event updated successfully".to_string()
    };
    axum::response::Html(format!(
        r#"<div class="px-4 py-3 bg-green-100 text-green-800 rounded-md text-sm">{}</div>"#,
        crate::web::escape_html(&msg),
    )).into_response()
}

pub async fn admin_delete_event(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Path(event_id): Path<String>,
    axum::Form(form): axum::Form<DeleteEventForm>,
) -> impl IntoResponse {
    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

    let id = match uuid::Uuid::parse_str(&event_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html("Invalid event ID".to_string()).into_response(),
    };

    let event = match state.service_context.event_repo.find_by_id(id).await {
        Ok(Some(e)) => e,
        Ok(None) => return axum::response::Html("Event not found".to_string()).into_response(),
        Err(_) => return axum::response::Html("Error loading event".to_string()).into_response(),
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
            // Hard-delete every occurrence after this one, then cap
            // the series at this row's start_time so the horizon job
            // doesn't re-materialize them.
            let cutoff = event.start_time;
            if let Err(e) = state.service_context.event_repo
                .delete_series_occurrences_after(sid, cutoff).await
            {
                return axum::response::Html(format!(
                    "Error ending series: {}", crate::web::escape_html(&e.to_string())
                )).into_response();
            }
            if let Err(e) = state.service_context.event_series_repo
                .set_until_date(sid, cutoff).await
            {
                tracing::error!("set_until_date failed for series {}: {}", sid, e);
            }
            state.service_context.audit_service.log(
                Some(current_user.member.id),
                "end_event_series",
                "event_series",
                &sid.to_string(),
                None,
                Some(&event.title),
                None,
            ).await;
            return axum::response::Redirect::to(
                &format!("/portal/admin/events/{}", id),
            ).into_response();
        }

        // delete_series: nuke the series row and all occurrences
        // (FK ON DELETE CASCADE drops every occurrence).
        if let Err(e) = state.service_context.event_series_repo.delete(sid).await {
            return axum::response::Html(format!(
                "Error deleting series: {}", crate::web::escape_html(&e.to_string())
            )).into_response();
        }
        state.service_context.audit_service.log(
            Some(current_user.member.id),
            "delete_event_series",
            "event_series",
            &sid.to_string(),
            None,
            Some(&event.title),
            None,
        ).await;
        return axum::response::Redirect::to("/portal/admin/events").into_response();
    }

    // Default: delete this single row, scope=="this".
    let image_to_delete = event.image_url.clone();
    match state.service_context.event_repo.delete(id).await {
        Ok(_) => {
            crate::web::uploads::delete_if_upload(
                &state.settings.server.uploads_path(),
                image_to_delete.as_deref(),
            ).await;
            state.service_context.audit_service.log(
                Some(current_user.member.id),
                "delete_event",
                "event",
                &id.to_string(),
                None,
                None,
                None,
            ).await;
            axum::response::Redirect::to("/portal/admin/events").into_response()
        }
        Err(e) => axum::response::Html(format!("Error deleting event: {}", crate::web::escape_html(&e.to_string()))).into_response(),
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
