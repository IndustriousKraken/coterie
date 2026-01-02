use askama::Template;
use axum::{
    extract::{State, Query},
    response::IntoResponse,
    Extension,
};
use serde::Deserialize;

use crate::{
    api::{
        middleware::auth::CurrentUser,
        state::AppState,
    },
    web::templates::{HtmlTemplate, UserInfo},
};
use super::is_admin;

#[derive(Template)]
#[template(path = "portal/events.html")]
pub struct EventsTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
}

pub async fn events_page(
    State(_state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {
    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let template = EventsTemplate {
        current_user: Some(user_info),
        is_admin: is_admin(&current_user.member),
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
    State(state): State<AppState>,
    Extension(_current_user): Extension<CurrentUser>,
    Query(query): Query<EventsListQuery>,
) -> impl IntoResponse {
    // Get upcoming events (past events not currently supported)
    let events = state.service_context.event_repo
        .list_upcoming(50)
        .await
        .unwrap_or_default();

    let now = chrono::Utc::now();

    // Filter events by type (past events not currently supported by repository)
    let filtered_events: Vec<_> = events.into_iter()
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
            </div>"#.to_string()
        );
    }

    let mut html = String::new();
    html.push_str(r#"<div class="space-y-4">"#);

    for event in filtered_events {
        let is_past = event.start_time < now;
        let type_badge_color = match format!("{:?}", event.event_type).as_str() {
            "Meeting" => "bg-blue-100 text-blue-800",
            "Workshop" => "bg-purple-100 text-purple-800",
            "CTF" => "bg-red-100 text-red-800",
            "Social" => "bg-green-100 text-green-800",
            "Training" => "bg-yellow-100 text-yellow-800",
            _ => "bg-gray-100 text-gray-800",
        };

        html.push_str(&format!(
            r#"<div class="bg-white rounded-lg shadow-sm p-6 {}">
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
                    <div class="text-right">
                        {}
                    </div>
                </div>
            </div>"#,
            if is_past { "opacity-60" } else { "" },
            type_badge_color,
            event.event_type,
            if is_past { r#"<span class="text-xs text-gray-500">Past event</span>"# } else { "" },
            event.title,
            event.description,
            event.start_time.format("%B %d, %Y"),
            event.start_time.format("%l:%M %p"),
            event.location.map(|l| format!(r#"<p>Location: {}</p>"#, l)).unwrap_or_default(),
            if !is_past {
                format!(
                    r#"<button hx-post="/portal/api/events/{}/rsvp"
                               hx-swap="outerHTML"
                               class="px-4 py-2 bg-blue-600 text-white text-sm rounded-md hover:bg-blue-700">
                        RSVP
                    </button>"#,
                    event.id
                )
            } else {
                String::new()
            }
        ));
    }

    html.push_str("</div>");
    axum::response::Html(html)
}
