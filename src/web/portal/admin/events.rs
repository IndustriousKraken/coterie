use askama::Template;
use axum::{
    extract::{State, Query, Path},
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
};
use crate::web::portal::is_admin;

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
    pub attendee_count: i64,
    pub is_past: bool,
    pub created_at: String,
    pub updated_at: String,
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
        attendee_count,
        is_past: event.start_time <= now,
        created_at: event.created_at.format("%b %d, %Y %H:%M").to_string(),
        updated_at: event.updated_at.format("%b %d, %Y %H:%M").to_string(),
    };

    HtmlTemplate(AdminEventDetailTemplate {
        current_user: Some(user_info),
        is_admin: true,
        csrf_token,
        event: detail,
    }).into_response()
}

#[derive(Template)]
#[template(path = "admin/event_new.html")]
pub struct AdminNewEventTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub csrf_token: String,
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

    HtmlTemplate(AdminNewEventTemplate {
        current_user: Some(user_info),
        is_admin: true,
        csrf_token,
    }).into_response()
}

#[derive(Debug, Deserialize)]
pub struct CreateEventForm {
    pub csrf_token: String,
    pub title: String,
    pub description: String,
    pub event_type: String,
    pub visibility: String,
    pub start_time: String,
    pub end_time: Option<String>,
    pub location: Option<String>,
    pub max_attendees: Option<i32>,
    pub rsvp_required: Option<String>,
}

pub async fn admin_create_event(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    axum::Form(form): axum::Form<CreateEventForm>,
) -> impl IntoResponse {
    use crate::domain::{Event, EventType, EventVisibility};

    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

    let event_type = match form.event_type.as_str() {
        "Meeting" => EventType::Meeting,
        "Workshop" => EventType::Workshop,
        "CTF" => EventType::CTF,
        "Social" => EventType::Social,
        "Training" => EventType::Training,
        _ => EventType::Meeting,
    };

    let visibility = match form.visibility.as_str() {
        "Public" => EventVisibility::Public,
        "MembersOnly" => EventVisibility::MembersOnly,
        "AdminOnly" => EventVisibility::AdminOnly,
        _ => EventVisibility::MembersOnly,
    };

    let start_time = match chrono::NaiveDateTime::parse_from_str(&form.start_time, "%Y-%m-%dT%H:%M") {
        Ok(dt) => chrono::DateTime::from_naive_utc_and_offset(dt, chrono::Utc),
        Err(_) => return axum::response::Html("Invalid start time".to_string()).into_response(),
    };

    let end_time = form.end_time
        .as_ref()
        .filter(|s| !s.is_empty())
        .and_then(|s| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M").ok())
        .map(|dt| chrono::DateTime::from_naive_utc_and_offset(dt, chrono::Utc));

    let event = Event {
        id: uuid::Uuid::new_v4(),
        title: form.title,
        description: form.description,
        event_type,
        visibility,
        start_time,
        end_time,
        location: form.location.filter(|s| !s.is_empty()),
        max_attendees: form.max_attendees,
        rsvp_required: form.rsvp_required.is_some(),
        created_by: current_user.member.id,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    match state.service_context.event_repo.create(event).await {
        Ok(created) => axum::response::Redirect::to(&format!("/portal/admin/events/{}", created.id)).into_response(),
        Err(e) => axum::response::Html(format!("Error creating event: {}", e)).into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateEventForm {
    pub csrf_token: String,
    pub title: String,
    pub description: String,
    pub event_type: String,
    pub visibility: String,
    pub start_time: String,
    pub end_time: Option<String>,
    pub location: Option<String>,
    pub max_attendees: Option<i32>,
    pub rsvp_required: Option<String>,
}

pub async fn admin_update_event(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Path(event_id): Path<String>,
    axum::Form(form): axum::Form<UpdateEventForm>,
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

    let event_type = match form.event_type.as_str() {
        "Meeting" => EventType::Meeting,
        "Workshop" => EventType::Workshop,
        "CTF" => EventType::CTF,
        "Social" => EventType::Social,
        "Training" => EventType::Training,
        _ => EventType::Meeting,
    };

    let visibility = match form.visibility.as_str() {
        "Public" => EventVisibility::Public,
        "MembersOnly" => EventVisibility::MembersOnly,
        "AdminOnly" => EventVisibility::AdminOnly,
        _ => EventVisibility::MembersOnly,
    };

    let start_time = match chrono::NaiveDateTime::parse_from_str(&form.start_time, "%Y-%m-%dT%H:%M") {
        Ok(dt) => chrono::DateTime::from_naive_utc_and_offset(dt, chrono::Utc),
        Err(_) => return axum::response::Html("Invalid start time".to_string()).into_response(),
    };

    let end_time = form.end_time
        .as_ref()
        .filter(|s| !s.is_empty())
        .and_then(|s| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M").ok())
        .map(|dt| chrono::DateTime::from_naive_utc_and_offset(dt, chrono::Utc));

    let updated_event = Event {
        id,
        title: form.title,
        description: form.description,
        event_type,
        visibility,
        start_time,
        end_time,
        location: form.location.filter(|s| !s.is_empty()),
        max_attendees: form.max_attendees,
        rsvp_required: form.rsvp_required.is_some(),
        created_by: existing.created_by,
        created_at: existing.created_at,
        updated_at: chrono::Utc::now(),
    };

    match state.service_context.event_repo.update(id, updated_event).await {
        Ok(_) => {
            axum::response::Html(r#"<div class="px-4 py-3 bg-green-100 text-green-800 rounded-md text-sm">Event updated successfully</div>"#.to_string()).into_response()
        }
        Err(e) => {
            axum::response::Html(format!(r#"<div class="px-4 py-3 bg-red-100 text-red-800 rounded-md text-sm">Error updating event: {}</div>"#, e)).into_response()
        }
    }
}

pub async fn admin_delete_event(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Path(event_id): Path<String>,
) -> impl IntoResponse {
    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

    let id = match uuid::Uuid::parse_str(&event_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html("Invalid event ID".to_string()).into_response(),
    };

    match state.service_context.event_repo.delete(id).await {
        Ok(_) => axum::response::Redirect::to("/portal/admin/events").into_response(),
        Err(e) => axum::response::Html(format!("Error deleting event: {}", e)).into_response(),
    }
}
