use askama::Template;
use axum::{
    extract::{State, Query},
    response::IntoResponse,
    Extension,
};
use serde::Serialize;

use crate::{
    api::{
        middleware::auth::CurrentUser,
        state::AppState,
        handlers::{
            events::{ListEventsQuery, list as list_events},
            payments::{ListPaymentsQuery, list_by_member as list_payments},
        },
    },
    domain::AttendanceStatus,
    web::templates::{HtmlTemplate, UserInfo},
};
use super::{MemberInfo, is_admin};

#[derive(Template)]
#[template(path = "dashboard/member.html")]
pub struct MemberDashboardTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub member: MemberInfo,
}

pub async fn member_dashboard(
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {
    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let member_info = MemberInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        full_name: current_user.member.full_name.clone(),
        email: current_user.member.email.clone(),
        status: format!("{:?}", current_user.member.status),
        membership_type: format!("{:?}", current_user.member.membership_type),
        joined_at: current_user.member.joined_at.format("%B %d, %Y").to_string(),
        dues_paid_until: current_user.member.dues_paid_until
            .map(|d| d.format("%B %d, %Y").to_string()),
    };

    let template = MemberDashboardTemplate {
        current_user: Some(user_info),
        is_admin: is_admin(&current_user.member),
        member: member_info,
    };

    HtmlTemplate(template)
}

// API endpoint for upcoming events
#[derive(Serialize)]
struct EventSummary {
    id: String,
    title: String,
    date: String,
    time: String,
    location: Option<String>,
    image_url: Option<String>,
    attending: bool,
}

pub async fn upcoming_events(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {
    // Use the existing events API to get upcoming events
    let query = ListEventsQuery {
        limit: Some(5),
        public_only: Some(false),
    };

    let events_result = list_events(
        State(state.clone()),
        Query(query),
        Some(Extension(current_user.clone())),
    ).await;

    let events = match events_result {
        Ok(json) => json.0,
        Err(_) => vec![],
    };

    // Transform to our summary format, checking attendance for each event
    let member_id = current_user.member.id;
    let mut event_summaries: Vec<EventSummary> = Vec::new();

    for event in events {
        let attending = state.service_context.event_repo
            .get_member_attendance_status(event.id, member_id)
            .await
            .ok()
            .flatten()
            .map(|s| matches!(s, AttendanceStatus::Registered))
            .unwrap_or(false);

        event_summaries.push(EventSummary {
            id: event.id.to_string(),
            title: event.title,
            date: event.start_time.format("%B %d, %Y").to_string(),
            time: event.start_time.format("%l:%M %p").to_string(),
            location: event.location,
            image_url: event.image_url,
            attending,
        });
    }

    // Return HTML fragment for HTMX
    let html = if event_summaries.is_empty() {
        r#"<p class="text-gray-500">No upcoming events</p>"#.to_string()
    } else {
        let mut html = String::from(r#"<div class="space-y-3">"#);
        for event in event_summaries {
            let image_html = event.image_url.as_ref().map(|url| {
                format!(r#"<img src="/{}" alt="" class="w-16 h-16 object-cover rounded flex-shrink-0">"#, url)
            }).unwrap_or_default();

            html.push_str(&format!(
                r#"
                <div class="border-l-4 border-blue-500 pl-3 flex gap-3">
                    {}
                    <div class="flex-1 min-w-0">
                        <h3 class="font-medium">{}</h3>
                        <p class="text-sm text-gray-600">{} at {}</p>
                        {}
                        <div class="mt-1">
                            {}
                        </div>
                    </div>
                </div>
                "#,
                image_html,
                event.title,
                event.date,
                event.time,
                event.location.map(|l| format!(r#"<p class="text-sm text-gray-600">📍 {}</p>"#, l)).unwrap_or_default(),
                if event.attending {
                    r#"<span class="text-xs text-green-600 font-medium">Attending</span>"#.to_string()
                } else {
                    format!(r#"<button hx-post="/portal/api/events/{}/rsvp" hx-swap="outerHTML" class="text-xs text-blue-600 hover:text-blue-800">RSVP</button>"#, event.id)
                }
            ));
        }
        html.push_str("</div>");
        html
    };

    axum::response::Html(html)
}

// API endpoint for recent payments
#[derive(Serialize)]
struct PaymentSummary {
    id: String,
    amount: String,
    status: String,
    date: String,
    description: String,
}

pub async fn recent_payments(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {
    use axum::extract::Path;

    // Use the existing payments API to get member's payments
    let query = ListPaymentsQuery {
        status: None,
        limit: Some(5),
    };

    let payments_result = list_payments(
        State(state.clone()),
        Path(current_user.member.id),
        Query(query),
        Extension(current_user.clone()),
    ).await;

    let payments = match payments_result {
        Ok(json) => json.0,
        Err(_) => vec![],
    };

    // Transform to our summary format
    let recent_payments: Vec<PaymentSummary> = payments.into_iter()
        .map(|p| PaymentSummary {
            id: p.id.to_string(),
            amount: format!("${:.2}", p.amount_cents as f64 / 100.0),
            status: format!("{:?}", p.status),
            date: p.created_at.format("%B %d, %Y").to_string(),
            description: if p.description.is_empty() {
                "Membership dues".to_string()
            } else {
                p.description
            },
        })
        .collect();

    // Return HTML fragment for HTMX
    let html = if recent_payments.is_empty() {
        r#"<p class="text-gray-500">No payment history</p>"#.to_string()
    } else {
        let mut html = String::from(r#"<div class="space-y-2">"#);
        for payment in recent_payments {
            let status_class = match payment.status.as_str() {
                "Completed" => "text-green-600",
                "Pending" => "text-yellow-600",
                "Failed" => "text-red-600",
                _ => "text-gray-600",
            };

            html.push_str(&format!(
                r#"
                <div class="flex justify-between items-center py-2 border-b">
                    <div>
                        <p class="text-sm font-medium">{}</p>
                        <p class="text-xs text-gray-500">{}</p>
                    </div>
                    <div class="text-right">
                        <p class="text-sm font-medium">{}</p>
                        <p class="text-xs {}">{}</p>
                    </div>
                </div>
                "#,
                payment.description,
                payment.date,
                payment.amount,
                status_class,
                payment.status
            ));
        }
        html.push_str("</div>");
        html
    };

    axum::response::Html(html)
}
