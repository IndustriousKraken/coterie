use askama::Template;
use axum::{
    extract::State,
    response::IntoResponse,
    Extension,
};
use serde::Serialize;

use crate::{
    api::{
        middleware::auth::{CurrentUser, SessionInfo},
        state::AppState,
    },
    domain::AttendanceStatus,
    web::templates::{BaseContext, HtmlTemplate},
};
use super::MemberInfo;

#[derive(Template)]
#[template(path = "dashboard/member.html")]
pub struct MemberDashboardTemplate {
    pub base: BaseContext,
    pub member: MemberInfo,
}

/// Async-loaded banner on every portal page. Shows a warning when dues
/// are past due but the member is still within grace period (status is
/// still Active). Returns empty HTML when dues are current or the
/// member is already Expired (their dedicated /portal/restore page
/// already tells them).
pub async fn dues_warning(
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {
    use crate::domain::MemberStatus;

    let member = &current_user.member;

    // Nothing to warn about for Honorary members or bypass-dues accounts.
    if member.status != MemberStatus::Active || member.bypass_dues {
        return axum::response::Html(String::new());
    }

    let now = chrono::Utc::now();
    let Some(due) = member.dues_paid_until else {
        return axum::response::Html(String::new());
    };

    if due > now {
        // Dues are current.
        return axum::response::Html(String::new());
    }

    // Past due but still Active — within grace period. Nudge them.
    let days_overdue = (now - due).num_days();
    let overdue_text = match days_overdue {
        0 => "today".to_string(),
        1 => "1 day ago".to_string(),
        n => format!("{} days ago", n),
    };

    let html = format!(
        r#"<div id="dues-banner" class="bg-amber-50 border-l-4 border-amber-500 px-4 py-3">
            <div class="max-w-7xl mx-auto flex items-center justify-between">
                <p class="text-sm text-amber-900">
                    <strong>Dues overdue.</strong>
                    Your membership dues lapsed {}. Please pay soon to avoid losing access.
                </p>
                <a href="/portal/payments/new"
                   class="ml-4 flex-shrink-0 text-sm font-medium text-amber-900 underline hover:text-amber-700">
                    Pay now
                </a>
            </div>
        </div>"#,
        crate::web::escape_html(&overdue_text),
    );
    axum::response::Html(html)
}

pub async fn member_dashboard(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session): Extension<SessionInfo>,
) -> impl IntoResponse {
    let member_info = MemberInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        full_name: current_user.member.full_name.clone(),
        email: current_user.member.email.clone(),
        status: current_user.member.status.as_str().to_string(),
        membership_type: current_user.member.membership_type.as_str().to_string(),
        joined_at: current_user.member.joined_at.format("%B %d, %Y").to_string(),
        dues_paid_until: current_user.member.dues_paid_until
            .map(|d| d.format("%B %d, %Y").to_string()),
    };

    let template = MemberDashboardTemplate {
        base: BaseContext::for_member(&state, &current_user, &session).await,
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
    // Authenticated members see both public and members-only events;
    // visibility filtering is per-event inside the template, not at
    // the repo layer.
    let events = state.service_context.event_repo
        .list_upcoming(5)
        .await
        .unwrap_or_default();

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
                format!(r#"<img src="/{}" alt="" class="w-16 h-16 object-cover rounded flex-shrink-0">"#, crate::web::escape_html(url))
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
                crate::web::escape_html(&event.title),
                event.date,
                event.time,
                event.location.map(|l| format!(r#"<p class="text-sm text-gray-600">📍 {}</p>"#, crate::web::escape_html(&l))).unwrap_or_default(),
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
    // Most-recent five payments for this member, regardless of status.
    let mut payments = state.service_context.payment_repo
        .find_by_member(current_user.member.id)
        .await
        .unwrap_or_default();
    payments.truncate(5);

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
                crate::web::escape_html(&p.description)
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
