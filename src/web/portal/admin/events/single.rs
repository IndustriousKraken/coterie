use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Multipart, Path, Query, State},
    response::{IntoResponse, Response},
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
    domain::{Event, EventType, EventVisibility},
    repository::{EventRepository, PaymentRepository},
    service::event_admin_service::{CreateEventInput, EventAdminService, UpdateEventInput},
    service::payment_admin_service::PaymentAdminService,
    service::settings_service::SettingsService,
    web::portal::admin::partials,
    web::templates::{BaseContext, HtmlTemplate},
    web::uploads::save_uploaded_file,
};

use super::{RosterRow, TypeOption};

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

/// The admin list's time-filter arms, in one function because they must
/// stay exact complements. Derived separately they would drift, and an
/// event in progress — now upcoming until it ends — would list under
/// BOTH `upcoming` and `past`.
fn matches_time_filter(filter: &str, event: &Event, now: chrono::DateTime<chrono::Utc>) -> bool {
    match filter {
        "upcoming" => event.is_upcoming(now),
        "past" => !event.is_upcoming(now),
        _ => true,
    }
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
            matches_time_filter(&time_filter, e, now)
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
        // Same predicate as the filter above, negated — a row shown under
        // `upcoming` must not also be badged "past".
        let is_past = !e.is_upcoming(now);

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
            is_past,
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
    /// Price as the form wants it — dollars, or empty for a free event
    /// so the field renders blank rather than "0".
    pub member_price_input: String,
    pub member_price_display: String,
    pub is_paid: bool,
    /// Same treatment for the guest price: blank field for a free event.
    pub guest_price_input: String,
    pub guest_price_display: String,
    pub guest_registration_enabled: bool,
    /// The shareable public registration URL, or `None` when the event
    /// isn't publicly registerable — the admin page shows the organizer
    /// exactly what they can paste, and nothing when there's nothing.
    pub registration_url: Option<String>,
    /// Attendees with their seat + money state. Empty for an event
    /// nobody has registered for.
    pub roster: Vec<RosterRow>,
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
    State(settings): State<Arc<Settings>>,
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
    let roster: Vec<RosterRow> = event_repo
        .roster(event.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(RosterRow::from_entry)
        .collect();

    let now = chrono::Utc::now();
    let is_past = !event.is_upcoming(now);
    let member_price_cents = event.member_price_cents;
    let guest_price_cents = event.guest_price_cents;
    // One home for the registerability rule (`Event::publicly_registerable`),
    // so the admin page can't disagree with the public one about whether
    // a URL exists.
    let registration_url = event.registration_url(&settings.server.base_url);

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
        member_price_input: if member_price_cents > 0 {
            format!("{:.2}", member_price_cents as f64 / 100.0)
        } else {
            String::new()
        },
        member_price_display: format!("${:.2}", member_price_cents as f64 / 100.0),
        is_paid: member_price_cents > 0,
        guest_price_input: if guest_price_cents > 0 {
            format!("{:.2}", guest_price_cents as f64 / 100.0)
        } else {
            String::new()
        },
        guest_price_display: if guest_price_cents > 0 {
            format!("${:.2}", guest_price_cents as f64 / 100.0)
        } else {
            "Free".to_string()
        },
        guest_registration_enabled: event.guest_registration_enabled,
        registration_url,
        roster,
        is_past,
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

/// Everything either admin event form can post, parsed once. The create
/// form sends the recurrence/series-pass fields, the update form sends
/// `edit_scope`/`remove_image`, and each handler simply ignores the
/// fields its own form never sends — parsing a field a handler does not
/// read changes nothing about what it does.
struct EventForm {
    title: String,
    description: String,
    event_type: EventType,
    visibility: EventVisibility,
    start_time: chrono::DateTime<chrono::Utc>,
    end_time: Option<chrono::DateTime<chrono::Utc>>,
    location: String,
    max_attendees: Option<i32>,
    rsvp_required: bool,
    // Raw price text; parsed by the caller so a blank field and a typed
    // "0" both land on 0 and only a real negative / over-cap value errors.
    member_price_str: String,
    guest_price_str: String,
    guest_registration_enabled: bool,
    image_url: Option<String>,
    // Recurrence form fields (create only). `repeat_kind` defaults to
    // "none" so an unchecked form behaves identically to the
    // pre-recurrence flow.
    repeat_kind: String,
    repeat_interval: u32,
    repeat_weekdays: Vec<String>,
    repeat_day: Option<u32>,
    repeat_weekday: String,
    repeat_ordinal: i32,
    repeat_until_str: String,
    // Series-pass pricing (create only). Only meaningful when a
    // recurrence was asked for; a one-off event has no series to price.
    series_member_price_str: String,
    series_guest_price_str: String,
    series_capacity: Option<i32>,
    series_guest_registration_enabled: bool,
    // Update only. For series occurrences: "this" (default),
    // "this_and_future". Ignored for one-off events.
    edit_scope: String,
    remove_image: bool,
}

impl EventForm {
    /// Drain the multipart body into the parsed superset. `Err` carries
    /// the rendered alert the handler returns as-is: an image upload
    /// that failed, or a start time we could not parse.
    async fn parse(
        multipart: &mut Multipart,
        uploads_dir: &str,
    ) -> std::result::Result<EventForm, Response> {
        let mut title = String::new();
        let mut description = String::new();
        let mut event_type_str = String::new();
        let mut visibility_str = String::new();
        let mut start_time_str = String::new();
        let mut end_time_str = String::new();
        let mut location = String::new();
        let mut max_attendees: Option<i32> = None;
        let mut rsvp_required = false;
        let mut member_price_str = String::new();
        let mut guest_price_str = String::new();
        // An unchecked checkbox sends no field at all, so absence is
        // `false` — the same shape `rsvp_required` uses.
        let mut guest_registration_enabled = false;
        let mut image_url: Option<String> = None;
        let mut repeat_kind = String::from("none");
        let mut repeat_interval: u32 = 1;
        let mut repeat_weekdays: Vec<String> = Vec::new();
        let mut repeat_day: Option<u32> = None;
        let mut repeat_weekday = String::from("mon");
        let mut repeat_ordinal: i32 = 1;
        let mut repeat_until_str = String::new();
        let mut series_member_price_str = String::new();
        let mut series_guest_price_str = String::new();
        let mut series_capacity: Option<i32> = None;
        let mut series_guest_registration_enabled = false;
        let mut edit_scope = String::from("this");
        let mut remove_image = false;

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
                "location" => location = field.text().await.unwrap_or_default(),
                "max_attendees" => {
                    if let Ok(text) = field.text().await {
                        max_attendees = text.parse().ok();
                    }
                }
                "rsvp_required" => {
                    rsvp_required = true;
                    let _ = field.text().await;
                }
                "member_price" => member_price_str = field.text().await.unwrap_or_default(),
                "guest_price" => guest_price_str = field.text().await.unwrap_or_default(),
                "guest_registration_enabled" => {
                    guest_registration_enabled = true;
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
                "series_member_price" => {
                    series_member_price_str = field.text().await.unwrap_or_default()
                }
                "series_guest_price" => {
                    series_guest_price_str = field.text().await.unwrap_or_default()
                }
                "series_capacity" => {
                    if let Ok(text) = field.text().await {
                        series_capacity = text.parse().ok();
                    }
                }
                "series_guest_registration_enabled" => {
                    series_guest_registration_enabled = true;
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
                                match save_uploaded_file(uploads_dir, &filename, &data).await {
                                    Ok(path) => image_url = Some(path),
                                    Err(e) => {
                                        return Err(partials::admin_alert(
                                            "error",
                                            &format!("Error uploading image: {}", e),
                                            false,
                                        )
                                        .into_response())
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

        // The `event-timezone` model stores the operator's naive local
        // wall-clock in a UTC container; the event's zone is what the read
        // paths use to derive the true instant. Do NOT turn this into a
        // real timezone conversion.
        let start_time =
            match chrono::NaiveDateTime::parse_from_str(&start_time_str, "%Y-%m-%dT%H:%M") {
                Ok(dt) => chrono::DateTime::from_naive_utc_and_offset(dt, chrono::Utc),
                Err(_) => {
                    return Err(
                        partials::admin_alert("error", "Invalid start time", false).into_response()
                    )
                }
            };

        let end_time = if end_time_str.is_empty() {
            None
        } else {
            chrono::NaiveDateTime::parse_from_str(&end_time_str, "%Y-%m-%dT%H:%M")
                .ok()
                .map(|dt| chrono::DateTime::from_naive_utc_and_offset(dt, chrono::Utc))
        };

        Ok(EventForm {
            title,
            description,
            event_type,
            visibility,
            start_time,
            end_time,
            location,
            max_attendees,
            rsvp_required,
            member_price_str,
            guest_price_str,
            guest_registration_enabled,
            image_url,
            repeat_kind,
            repeat_interval,
            repeat_weekdays,
            repeat_day,
            repeat_weekday,
            repeat_ordinal,
            repeat_until_str,
            series_member_price_str,
            series_guest_price_str,
            series_capacity,
            series_guest_registration_enabled,
            edit_scope,
            remove_image,
        })
    }
}

pub async fn admin_create_event(
    State(settings): State<Arc<Settings>>,
    State(settings_service): State<Arc<SettingsService>>,
    State(event_admin_service): State<Arc<EventAdminService>>,
    Extension(current_user): Extension<CurrentUser>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let form = match EventForm::parse(&mut multipart, &settings.server.uploads_path()).await {
        Ok(f) => f,
        Err(r) => return r,
    };

    // Build the recurrence rule, if the admin asked for one. The
    // service decides series-vs-single by inspecting input.recurrence.
    let recurrence = if form.repeat_kind != "none" && !form.repeat_kind.is_empty() {
        match build_recurrence(
            &form.repeat_kind,
            form.repeat_interval,
            &form.repeat_weekdays,
            form.repeat_day,
            &form.repeat_weekday,
            form.repeat_ordinal,
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
        parse_until(&form.repeat_until_str)
    } else {
        None
    };

    // Freeze the event's zone from the current org setting. The naive
    // form input is stored as-is (no conversion); the zone is what makes
    // the public/iCal read path derive the correct instant.
    let timezone = settings_service.org_timezone().await.name().to_string();

    let member_price_cents = match parse_price(&form.member_price_str) {
        Ok(cents) => cents,
        Err(msg) => return partials::admin_alert("error", msg, false).into_response(),
    };
    // Guest registration at a zero price is a valid, supported
    // combination (the free workshop) — the flag and the price are not
    // cross-validated against each other on purpose.
    let guest_price_cents = match parse_price(&form.guest_price_str) {
        Ok(cents) => cents,
        Err(msg) => return partials::admin_alert("error", msg, false).into_response(),
    };

    // Pass pricing is only read when a recurrence was requested — a
    // one-off event has no series, and silently carrying a price into
    // one would be a charge nobody could ever be enrolled against.
    let series_pricing = if recurrence.is_some() {
        match parse_series_pricing(
            &form.series_member_price_str,
            &form.series_guest_price_str,
            form.series_capacity,
            form.series_guest_registration_enabled,
        ) {
            Ok(p) => p,
            Err(msg) => return partials::admin_alert("error", msg, false).into_response(),
        }
    } else {
        Default::default()
    };

    let input = CreateEventInput {
        title: form.title,
        description: form.description,
        event_type: form.event_type,
        event_type_id: None,
        visibility: form.visibility,
        start_time: form.start_time,
        end_time: form.end_time,
        timezone,
        location: if form.location.is_empty() {
            None
        } else {
            Some(form.location)
        },
        max_attendees: form.max_attendees,
        rsvp_required: form.rsvp_required,
        member_price_cents,
        guest_price_cents,
        guest_registration_enabled: form.guest_registration_enabled,
        image_url: form.image_url,
        recurrence,
        recurrence_until,
        series_pricing,
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

/// Parse an admin form price field (member or guest) into cents.
///
/// A blank field and a typed `0` BOTH mean free and both store `0` —
/// neither is an error, and neither is rewritten to NULL. Only a
/// negative (which expresses no coherent intent) or an over-cap value
/// is rejected. Accepts dollars with an optional `$` and up to two
/// decimal places, which is what an admin actually types.
pub(super) fn parse_price(raw: &str) -> std::result::Result<i64, &'static str> {
    let cleaned = raw.trim().trim_start_matches('$').replace(',', "");
    if cleaned.is_empty() {
        return Ok(0);
    }
    let dollars: f64 = cleaned.parse().map_err(|_| "Invalid price")?;
    if !dollars.is_finite() {
        return Err("Invalid price");
    }
    if dollars < 0.0 {
        return Err("Price can't be negative");
    }
    let cents = (dollars * 100.0).round() as i64;
    if cents > crate::domain::MAX_PAYMENT_CENTS {
        return Err("Price exceeds the cap on a single payment");
    }
    Ok(cents)
}

/// Parse the admin form's series-pass fields into a
/// [`crate::domain::SeriesPassPricing`]. Prices go through the same
/// `parse_price` an event's own prices do, so blank and `0` both mean
/// free and only a negative or over-cap value is an error. A capacity of
/// zero or below is treated as "no limit" rather than "nobody may
/// enroll", which is what an operator clearing the field means.
///
/// The bounded-class rule (`until_date` required) is NOT checked here:
/// it lives in `validate_pass_pricing`, which the create/update service
/// paths call, so a hand-posted form can't route around it.
fn parse_series_pricing(
    member_price: &str,
    guest_price: &str,
    capacity: Option<i32>,
    guest_registration_enabled: bool,
) -> std::result::Result<crate::domain::SeriesPassPricing, &'static str> {
    Ok(crate::domain::SeriesPassPricing {
        member_price_cents: parse_price(member_price)?,
        guest_price_cents: parse_price(guest_price)?,
        guest_registration_enabled,
        max_enrollments: capacity.filter(|c| *c > 0),
    })
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

    let form = match EventForm::parse(&mut multipart, &settings.server.uploads_path()).await {
        Ok(f) => f,
        Err(r) => return r,
    };

    // Determine final image_url: new upload > remove > keep existing.
    // Also capture what (if anything) we need to delete from disk.
    let old_image = existing.image_url.clone();
    let image_url = if form.image_url.is_some() {
        form.image_url
    } else if form.remove_image {
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

    // Changing the price never re-bills an existing attendee — their
    // recorded payment is a separate, already-settled row.
    let member_price_cents = match parse_price(&form.member_price_str) {
        Ok(cents) => cents,
        Err(msg) => return partials::admin_alert("error", msg, false).into_response(),
    };
    let guest_price_cents = match parse_price(&form.guest_price_str) {
        Ok(cents) => cents,
        Err(msg) => return partials::admin_alert("error", msg, false).into_response(),
    };

    let input = UpdateEventInput {
        title: form.title,
        description: form.description,
        event_type: form.event_type,
        event_type_id: existing.event_type_id,
        visibility: form.visibility,
        start_time: form.start_time,
        end_time: form.end_time,
        location: if form.location.is_empty() {
            None
        } else {
            Some(form.location)
        },
        max_attendees: form.max_attendees,
        rsvp_required: form.rsvp_required,
        member_price_cents,
        guest_price_cents,
        guest_registration_enabled: form.guest_registration_enabled,
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
    if form.edit_scope == "this_and_future" {
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

    let msg = if form.edit_scope == "this_and_future" {
        format!(
            "Event updated. {} future occurrences also updated.",
            future_count.saturating_sub(1)
        )
    } else {
        "Event updated successfully".to_string()
    };
    partials::admin_alert("success", &msg, false).into_response()
}

/// Does ANY occurrence of `series_id` still carry a `Completed` event
/// fee, from a member or a guest? An `Err` means the question could not
/// be answered — callers must treat that as a refusal, not a "no".
async fn series_has_paid_occurrence(
    event_repo: &dyn EventRepository,
    payment_repo: &dyn PaymentRepository,
    series_id: uuid::Uuid,
) -> crate::error::Result<bool> {
    for occ in event_repo.list_series_occurrences(series_id).await? {
        if !payment_repo
            .list_completed_event_fees(occ.id)
            .await?
            .is_empty()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Delete an event.
///
/// For a paid event the roster's `Completed` event fees are refunded
/// FIRST, and a failing refund aborts the delete. `event_attendance`
/// cascades on event delete, so the reverse order would vaporize the
/// record of who was owed money while the charges stood.
pub async fn admin_delete_event(
    State(settings): State<Arc<Settings>>,
    State(event_repo): State<Arc<dyn EventRepository>>,
    State(event_admin_service): State<Arc<EventAdminService>>,
    State(payment_admin_service): State<Arc<PaymentAdminService>>,
    State(payment_repo): State<Arc<dyn PaymentRepository>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(event_id): Path<String>,
    headers: axum::http::HeaderMap,
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

    let ip = crate::api::state::client_ip(&headers, settings.server.trust_forwarded_for());

    let bulk_scope = scope == "end_series" || scope == "delete_series";
    if let Some(sid) = series_id.filter(|_| bulk_scope) {
        // Bulk series removal drops occurrences this handler never sees,
        // so it can't refund their PER-OCCURRENCE attendees — only the
        // per-occurrence path below can. Refuse whenever ANY occurrence
        // of the series still carries a `Completed` event fee, member or
        // guest: the two price columns are independent (free for members,
        // priced for the public is a supported shape) and a price that
        // was later zeroed doesn't return the money already taken, so
        // only the payments themselves answer the question. A lookup
        // that fails answers nothing, so it refuses too — an unreadable
        // roster must not authorise destroying one. A class pass is a
        // different matter: it is one payment at series scope, and the
        // sweep for it is right below.
        match series_has_paid_occurrence(&*event_repo, &*payment_repo, sid).await {
            Ok(false) => {}
            Ok(true) => {
                return partials::admin_alert(
                    "error",
                    "Attendees have paid for individual sessions of this series. Delete those \
                     sessions one at a time so each one's attendees are refunded first.",
                    false,
                )
                .into_response();
            }
            Err(e) => {
                tracing::error!("series {} paid-occurrence check failed: {}", sid, e);
                return partials::admin_alert(
                    "error",
                    "Couldn't check this series' sessions for paid attendees, so nothing was \
                     deleted. Try again.",
                    false,
                )
                .into_response();
            }
        }

        // Refund every class pass BEFORE deleting. Occurrences and their
        // attendance cascade on series delete, so the reverse order would
        // destroy the roster while the charges stood. A class that can't
        // be fully refunded stays alive, visible, and fixable.
        //
        // Ending a series deletes future occurrences too, so it refunds on
        // the same terms — a pass-holder whose remaining sessions all
        // vanish is owed their money either way.
        match payment_admin_service
            .refund_all_series_passes(current_user.member.id, sid, ip)
            .await
        {
            Ok(0) => {}
            Ok(n) => tracing::info!(
                "Refunded {} series passes before altering series {}",
                n,
                sid
            ),
            Err(e) => {
                return partials::admin_alert(
                    "error",
                    &format!(
                        "Series NOT changed — a refund failed: {}. Fix the refund, then retry.",
                        e.user_message(),
                    ),
                    false,
                )
                .into_response();
            }
        }

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

    // Refund before delete. An event that can't be fully refunded stays
    // alive, visible, and fixable rather than becoming an invisible pile
    // of unreturned charges.
    match payment_admin_service
        .refund_all_event_fees(current_user.member.id, id, ip)
        .await
    {
        Ok(0) => {}
        Ok(n) => tracing::info!(
            "Refunded {} event-fee payments before deleting event {}",
            n,
            id
        ),
        Err(e) => {
            return partials::admin_alert(
                "error",
                &format!(
                    "Event NOT deleted — a refund failed: {}. Fix the refund, then delete.",
                    e.user_message(),
                ),
                false,
            )
            .into_response();
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

#[cfg(test)]
mod tests {
    use super::parse_price;
    use crate::domain::MAX_PAYMENT_CENTS;

    // A blank field and a typed 0 both mean "free" and both store 0.
    // Neither is an error — an admin shouldn't have to learn a storage
    // convention to price an event at nothing.
    #[test]
    fn blank_and_zero_both_store_zero() {
        for raw in ["", "   ", "0", "0.00", "$0"] {
            assert_eq!(parse_price(raw), Ok(0), "input {raw:?}");
        }
    }

    #[test]
    fn dollars_convert_to_cents() {
        assert_eq!(parse_price("30"), Ok(3000));
        assert_eq!(parse_price("12.50"), Ok(1250));
        assert_eq!(parse_price("$1,200"), Ok(120_000));
    }

    // Unlike 0, a negative expresses no coherent intent.
    #[test]
    fn negative_is_rejected() {
        assert!(parse_price("-1").is_err());
        assert!(parse_price("-0.01").is_err());
    }

    #[test]
    fn over_cap_is_rejected() {
        let over = (MAX_PAYMENT_CENTS / 100) + 1;
        assert!(parse_price(&over.to_string()).is_err());
        // Exactly at the cap is fine.
        assert_eq!(
            parse_price(&(MAX_PAYMENT_CENTS / 100).to_string()),
            Ok(MAX_PAYMENT_CENTS),
        );
    }

    #[test]
    fn garbage_is_rejected_rather_than_silently_zeroed() {
        assert!(parse_price("free").is_err());
        assert!(parse_price("1o").is_err());
    }

    // 3.9 — `upcoming` and `past` must partition the list: every event
    // lands under exactly one of them, including one in progress. Two
    // separately-derived arms would list an in-progress event under both.
    mod time_filter {
        use super::super::matches_time_filter;
        use crate::domain::{Event, EventType, EventVisibility};
        use chrono::{DateTime, Duration, Utc};
        use uuid::Uuid;

        fn event(offset_from_now: Duration, duration: Option<Duration>) -> Event {
            let start = Utc::now() + offset_from_now;
            Event {
                id: Uuid::new_v4(),
                title: "T".into(),
                description: String::new(),
                event_type: EventType::Meeting,
                event_type_id: None,
                visibility: EventVisibility::Public,
                // `timezone` is UTC, so the stored wall-clock container
                // and the derived instant coincide.
                start_time: start,
                end_time: duration.map(|d| start + d),
                timezone: "UTC".into(),
                location: None,
                max_attendees: None,
                rsvp_required: false,
                member_price_cents: 0,
                guest_price_cents: 0,
                guest_registration_enabled: false,
                image_url: None,
                created_by: Uuid::new_v4(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                series_id: None,
                occurrence_index: None,
            }
        }

        #[test]
        fn upcoming_and_past_partition_every_event() {
            let h = Duration::hours(1);
            let fixture = [
                ("future", event(h * 24, Some(h))),
                ("in progress", event(-h, Some(h * 2))),
                ("ended", event(-h * 3, Some(h))),
                ("no end, inside grace", event(-Duration::minutes(30), None)),
                ("no end, past grace", event(-h * 3, None)),
            ];
            let now: DateTime<Utc> = Utc::now();

            for (label, e) in &fixture {
                let up = matches_time_filter("upcoming", e, now);
                let past = matches_time_filter("past", e, now);
                assert_ne!(up, past, "`{label}` must land under exactly one filter");
                // An unrecognized filter is the unfiltered list.
                assert!(matches_time_filter("", e, now), "`{label}` unfiltered");
            }

            // ...and the arms are on the right side of the boundary.
            let expected_upcoming = ["future", "in progress", "no end, inside grace"];
            for (label, e) in &fixture {
                assert_eq!(
                    matches_time_filter("upcoming", e, now),
                    expected_upcoming.contains(label),
                    "`{label}` is on the wrong side of upcoming",
                );
            }
        }
    }
}

/// The series-scope delete guard, exercised through the handler itself:
/// the bulk scopes drop occurrences the handler never sees, so the only
/// safe answer when ANY of them still holds a `Completed` event fee is
/// "no". Kept in its own module because it needs a live pool and the
/// whole service graph, unlike the pure `parse_price` tests above.
#[cfg(test)]
mod series_delete_guard_tests {
    use std::sync::Arc;
    use std::time::Duration;

    use axum::{
        extract::{Path, State},
        response::IntoResponse,
        Extension,
    };
    use chrono::{DateTime, Datelike, Utc, Weekday};
    use sqlx::{Executor, SqlitePool};
    use uuid::Uuid;

    use super::{admin_delete_event, DeleteEventForm};
    use crate::{
        api::{
            middleware::auth::CurrentUser,
            state::{MoneyLimiter, RateLimiter},
        },
        config::{AuthConfig, DatabaseConfig, ServerConfig, Settings},
        domain::{
            Attendee, CreateMemberRequest, Event, EventType, EventVisibility, Payer, Payment,
            PaymentKind, PaymentMethod, PaymentStatus, Recurrence, WeekdayCode,
        },
        integrations::IntegrationManager,
        payments::StripeHandle,
        repository::{
            EventRepository, EventSeriesRepository, MemberRepository, PaymentRepository,
            SeriesEnrollmentRepository, SqliteEventRepository, SqliteEventSeriesRepository,
            SqliteMemberRepository, SqlitePaymentRepository, SqliteSeriesEnrollmentRepository,
        },
        service::{
            audit_service::AuditService,
            event_admin_service::{CreateEventInput, EventAdminService},
            payment_admin_service::PaymentAdminService,
            recurring_event_service::RecurringEventService,
        },
    };

    struct Harness {
        pool: SqlitePool,
        settings: Arc<Settings>,
        event_repo: Arc<dyn EventRepository>,
        payment_repo: Arc<dyn PaymentRepository>,
        event_admin: Arc<EventAdminService>,
        payment_admin: Arc<PaymentAdminService>,
        admin: CurrentUser,
    }

    async fn build_harness() -> Harness {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .after_connect(|conn, _| {
                Box::pin(async move {
                    conn.execute("PRAGMA foreign_keys = ON").await?;
                    Ok(())
                })
            })
            .connect("sqlite::memory:")
            .await
            .expect(":memory:");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrate");

        let event_repo: Arc<dyn EventRepository> =
            Arc::new(SqliteEventRepository::new(pool.clone()));
        let series_repo: Arc<dyn EventSeriesRepository> =
            Arc::new(SqliteEventSeriesRepository::new(pool.clone()));
        let payment_repo: Arc<dyn PaymentRepository> =
            Arc::new(SqlitePaymentRepository::new(pool.clone()));
        let enrollment_repo: Arc<dyn SeriesEnrollmentRepository> =
            Arc::new(SqliteSeriesEnrollmentRepository::new(pool.clone()));
        let audit = Arc::new(AuditService::new(pool.clone()));
        let integrations = Arc::new(IntegrationManager::new());
        let recurring = Arc::new(RecurringEventService::new(
            event_repo.clone(),
            series_repo.clone(),
            pool.clone(),
        ));

        let event_admin = Arc::new(EventAdminService::new(
            event_repo.clone(),
            series_repo,
            recurring,
            audit.clone(),
            integrations.clone(),
        ));
        // No Stripe client: nothing on these paths should reach a
        // gateway, and a refund that tried to would fail loudly.
        let payment_admin = Arc::new(PaymentAdminService::new(
            payment_repo.clone(),
            event_repo.clone(),
            enrollment_repo,
            Arc::new(StripeHandle::preloaded(None, None)),
            audit,
            integrations,
            MoneyLimiter(RateLimiter::new(1000, Duration::from_secs(60))),
        ));

        let member = SqliteMemberRepository::new(pool.clone())
            .create(CreateMemberRequest {
                email: format!("admin-{}@example.com", Uuid::new_v4()),
                username: format!("u_{}", Uuid::new_v4().simple()),
                full_name: "Test Admin".to_string(),
                password: "p4ssword_long_enough".to_string(),
                membership_type_id: None,
                ..Default::default()
            })
            .await
            .unwrap();

        Harness {
            pool,
            settings: Arc::new(test_settings()),
            event_repo,
            payment_repo,
            event_admin,
            payment_admin,
            admin: CurrentUser { member },
        }
    }

    fn test_settings() -> Settings {
        Settings {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 0,
                base_url: "http://127.0.0.1".to_string(),
                data_dir: "./data".to_string(),
                uploads_dir: None,
                secure_cookies: Some(false),
                cors_origins: None,
                trust_forwarded_for: Some(false),
            },
            database: DatabaseConfig {
                url: "sqlite::memory:".to_string(),
                max_connections: 1,
            },
            auth: AuthConfig {
                session_secret: "test-session-secret-please-ignore".to_string(),
                session_duration_hours: 24,
                totp_issuer: "Coterie Test".to_string(),
            },
            stripe: Default::default(),
            integrations: Default::default(),
            seed: Default::default(),
        }
    }

    /// Next Tuesday at 18:00 UTC, strictly after tomorrow — the weekly
    /// rule requires the anchor to fall on the day it repeats.
    fn next_tuesday_anchor() -> DateTime<Utc> {
        let start = Utc::now() + chrono::Duration::days(1);
        let days = (Weekday::Tue.num_days_from_monday() as i64
            - start.weekday().num_days_from_monday() as i64)
            .rem_euclid(7);
        (start.date_naive() + chrono::Duration::days(days))
            .and_hms_opt(18, 0, 0)
            .unwrap()
            .and_utc()
    }

    /// A weekly public workshop: free for members, $25 for guests —
    /// the combination the spec calls out as supported, and the one the
    /// old member-price-only guard waved through.
    async fn make_series(h: &Harness, member_price_cents: i64, guest_price_cents: i64) -> Event {
        let start = next_tuesday_anchor();
        let input = CreateEventInput {
            title: "Lockpicking 101".to_string(),
            description: "Bring a padlock".to_string(),
            event_type: EventType::Workshop,
            event_type_id: None,
            visibility: EventVisibility::Public,
            start_time: start,
            end_time: None,
            timezone: "UTC".to_string(),
            location: None,
            max_attendees: None,
            rsvp_required: true,
            member_price_cents,
            guest_price_cents,
            guest_registration_enabled: true,
            image_url: None,
            recurrence: Some(Recurrence::WeeklyByDay {
                interval: 1,
                weekdays: vec![WeekdayCode::Tue],
            }),
            recurrence_until: Some(start + chrono::Duration::weeks(8)),
            series_pricing: Default::default(),
        };
        h.event_admin
            .create(h.admin.member.id, input)
            .await
            .unwrap()
    }

    /// Seat `attendee` on `event_id` with a `Completed` event fee, the
    /// way a real registration leaves the two rows.
    async fn seat_and_charge(h: &Harness, event_id: Uuid, payer: Payer, attendee: Attendee) {
        let now = Utc::now();
        let payment = h
            .payment_repo
            .create(Payment {
                id: Uuid::new_v4(),
                payer,
                amount_cents: 2500,
                currency: "USD".to_string(),
                status: PaymentStatus::Completed,
                payment_method: PaymentMethod::Manual,
                kind: PaymentKind::EventFee { event_id },
                external_id: None,
                description: "Event registration — Lockpicking 101".to_string(),
                paid_at: Some(now),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        h.event_repo
            .register_attendance(event_id, &attendee)
            .await
            .unwrap();
        h.event_repo
            .link_payment(event_id, &attendee, payment.id)
            .await
            .unwrap();
    }

    async fn post_delete(h: &Harness, event_id: Uuid, scope: &str) -> String {
        let resp = admin_delete_event(
            State(h.settings.clone()),
            State(h.event_repo.clone()),
            State(h.event_admin.clone()),
            State(h.payment_admin.clone()),
            State(h.payment_repo.clone()),
            Extension(h.admin.clone()),
            Path(event_id.to_string()),
            axum::http::HeaderMap::new(),
            axum::Form(DeleteEventForm {
                scope: Some(scope.to_string()),
                csrf_token: String::new(),
            }),
        )
        .await
        .into_response();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8_lossy(&body).to_string()
    }

    async fn occurrence_count(h: &Harness, series_id: Uuid) -> i64 {
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM events WHERE series_id = ?")
            .bind(series_id.to_string())
            .fetch_one(&h.pool)
            .await
            .unwrap();
        count.0
    }

    async fn series_exists(h: &Harness, series_id: Uuid) -> bool {
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM event_series WHERE id = ?")
            .bind(series_id.to_string())
            .fetch_one(&h.pool)
            .await
            .unwrap();
        count.0 == 1
    }

    // A guest fee is a `Completed` event-fee payment like any other, so
    // the bulk delete that would cascade its roster away has to refuse —
    // even though the member price is 0 and the old guard saw nothing.
    #[tokio::test]
    async fn series_delete_refused_when_a_guest_paid_an_occurrence() {
        let h = build_harness().await;
        let anchor = make_series(&h, 0, 2500).await;
        let series_id = anchor.series_id.unwrap();
        let before = occurrence_count(&h, series_id).await;
        assert!(before > 1, "expected a materialized series");

        // The guest bought a seat on a LATER night, not the one the
        // admin happens to have open.
        let occurrences = h
            .event_repo
            .list_series_occurrences(series_id)
            .await
            .unwrap();
        let paid_night = occurrences.last().unwrap().id;
        seat_and_charge(
            &h,
            paid_night,
            Payer::PublicDonor {
                name: "Ada Lovelace".to_string(),
                email: "ada@example.com".to_string(),
            },
            Attendee::Guest {
                name: "Ada Lovelace".to_string(),
                email: "ada@example.com".to_string(),
            },
        )
        .await;

        let body = post_delete(&h, anchor.id, "delete_series").await;
        assert!(
            body.contains("paid for individual sessions"),
            "expected the refusal alert, got: {body}"
        );
        assert!(series_exists(&h, series_id).await, "series row survived");
        assert_eq!(
            occurrence_count(&h, series_id).await,
            before,
            "no occurrence may be dropped by a refused delete"
        );
        let seats: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM event_attendance WHERE event_id = ?")
                .bind(paid_night.to_string())
                .fetch_one(&h.pool)
                .await
                .unwrap();
        assert_eq!(seats.0, 1, "the roster the refund needs is intact");
    }

    // The guard keys on payments, not on the price column: a series
    // priced for guests but never bought is still bulk-deletable.
    #[tokio::test]
    async fn series_delete_allowed_when_no_occurrence_was_paid() {
        let h = build_harness().await;
        let anchor = make_series(&h, 0, 2500).await;
        let series_id = anchor.series_id.unwrap();
        assert!(occurrence_count(&h, series_id).await > 1);

        let body = post_delete(&h, anchor.id, "delete_series").await;
        assert!(
            !body.contains("paid for individual sessions"),
            "a priced-but-unsold series must not be refused, got: {body}"
        );
        assert!(!series_exists(&h, series_id).await, "series row deleted");
        assert_eq!(occurrence_count(&h, series_id).await, 0);
    }

    // The member case the old guard covered stays covered under the new
    // rule — and `end_series` is refused on the same terms as a delete,
    // because it drops future occurrences the same way.
    #[tokio::test]
    async fn end_series_refused_when_a_member_paid_an_occurrence() {
        let h = build_harness().await;
        let anchor = make_series(&h, 2500, 0).await;
        let series_id = anchor.series_id.unwrap();
        let before = occurrence_count(&h, series_id).await;

        let occurrences = h
            .event_repo
            .list_series_occurrences(series_id)
            .await
            .unwrap();
        let paid_night = occurrences.last().unwrap().id;
        let member_id = h.admin.member.id;
        seat_and_charge(
            &h,
            paid_night,
            Payer::Member(member_id),
            Attendee::Member(member_id),
        )
        .await;

        let body = post_delete(&h, anchor.id, "end_series").await;
        assert!(
            body.contains("paid for individual sessions"),
            "expected the refusal alert, got: {body}"
        );
        assert_eq!(
            occurrence_count(&h, series_id).await,
            before,
            "ending a series must not drop a paid night either"
        );
    }
}
