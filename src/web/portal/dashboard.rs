use std::sync::Arc;

use askama::Template;
use axum::{extract::State, response::IntoResponse, Extension};
use serde::Serialize;

use super::{events::render_rsvp_button, MemberInfo};
use crate::{
    api::middleware::auth::{CurrentUser, SessionInfo},
    auth::CsrfService,
    domain::AttendanceStatus,
    repository::{EventRepository, PaymentRepository},
    service::membership_type_service::MembershipTypeService,
    service::settings_service::SettingsService,
    web::templates::{filters, BaseContext, HtmlTemplate},
};

#[derive(Template)]
#[template(path = "dashboard/member.html")]
pub struct MemberDashboardTemplate {
    pub base: BaseContext,
    pub member: MemberInfo,
    /// Whether the member-proposal-submissions capability is enabled.
    /// The dashboard is the entry point to submissions; when off, no
    /// link is shown (and the routes 404), so an org that hasn't opted
    /// in has no submission surface at all.
    pub submissions_enabled: bool,
}

/// Async-loaded banner on every portal page. Shows a warning when dues
/// are past due but the member is still within grace period (status is
/// still Active). Returns empty HTML when dues are current or the
/// member is already Expired (their dedicated /portal/restore page
/// already tells them).
pub async fn dues_warning(Extension(current_user): Extension<CurrentUser>) -> impl IntoResponse {
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
    State(membership_type_service): State<Arc<MembershipTypeService>>,
    State(settings_service): State<Arc<SettingsService>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session): Extension<SessionInfo>,
) -> impl IntoResponse {
    let membership_type_name = membership_type_service
        .get(current_user.member.membership_type_id)
        .await
        .ok()
        .flatten()
        .map(|mt| mt.name)
        .unwrap_or_else(|| "(unknown)".to_string());

    let member_info = MemberInfo {
        id: current_user.member.id,
        username: current_user.member.username.clone(),
        full_name: current_user.member.full_name.clone(),
        email: current_user.member.email.clone(),
        status: current_user.member.status,
        membership_type: membership_type_name,
        joined_at: current_user.member.joined_at,
        dues_paid_until: current_user.member.dues_paid_until,
    };

    let submissions_enabled = settings_service
        .get_bool("submissions.enabled")
        .await
        .unwrap_or(false);

    let template = MemberDashboardTemplate {
        base: BaseContext::for_member(&csrf_service, &current_user, &session).await,
        member: member_info,
        submissions_enabled,
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
    // Full RSVP status (not just a bool) so we can render the shared
    // register/cancel control — a member registered elsewhere must be able to
    // cancel from here after a reload, matching the Events page.
    rsvp: Option<AttendanceStatus>,
    /// Drives the control's label ("RSVP" vs "Register — $30"), same
    /// rule as the Events page.
    member_price_cents: i64,
}

pub async fn upcoming_events(
    State(event_repo): State<Arc<dyn EventRepository>>,
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {
    // Authenticated members see both public and members-only events;
    // visibility filtering is per-event inside the template, not at
    // the repo layer.
    let events = event_repo.list_upcoming(5).await.unwrap_or_default();

    // Transform to our summary format, checking attendance for each event
    let member_id = current_user.member.id;
    let mut event_summaries: Vec<EventSummary> = Vec::new();

    for event in events {
        let rsvp = event_repo
            .get_member_attendance_status(event.id, member_id)
            .await
            .ok()
            .flatten();

        // Wall-clock + zone abbr, so a remote member reads the right
        // local time (server-rendered; no browser conversion).
        let time = format!(
            "{} {}",
            event.start_time.format("%l:%M %p"),
            event.zone_abbr()
        );

        event_summaries.push(EventSummary {
            id: event.id.to_string(),
            title: event.title,
            date: event.start_time.format("%B %d, %Y").to_string(),
            time,
            location: event.location,
            image_url: event.image_url,
            rsvp,
            member_price_cents: event.member_price_cents,
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
                event
                    .location
                    .map(|l| format!(
                        r#"<p class="text-sm text-gray-600">📍 {}</p>"#,
                        crate::web::escape_html(&l)
                    ))
                    .unwrap_or_default(),
                // Shared control from the Events page: self-wrapped in
                // div.text-right with hx-target="closest div.text-right", so the
                // RSVP <-> cancel toggle works in both directions, including for a
                // member who registered elsewhere and then reloaded the dashboard.
                render_rsvp_button(&event.id, event.rsvp.as_ref(), event.member_price_cents)
            ));
        }
        html.push_str("</div>");
        html
    };

    axum::response::Html(html)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        routing::get,
        Router,
    };
    use chrono::Utc;
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::{
        domain::{CreateMemberRequest, Event, EventType, EventVisibility, Member},
        repository::{MemberRepository, SqliteEventRepository, SqliteMemberRepository},
    };

    async fn migrated_pool() -> sqlx::SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    // Regression for #110: a member already registered for an event (e.g. from
    // the Events page) must be able to cancel it from the dashboard's Upcoming
    // Events widget after a reload. The attending state must render the shared
    // cancel control rooted in div.text-right — not a dead <span>Attending</span>.
    #[tokio::test]
    async fn dashboard_attending_fragment_has_cancel_control() {
        let pool = migrated_pool().await;

        let member_repo: Arc<dyn MemberRepository> =
            Arc::new(SqliteMemberRepository::new(pool.clone()));
        let member: Member = member_repo
            .create(CreateMemberRequest {
                email: "member@example.com".to_string(),
                username: "member".to_string(),
                full_name: "Member".to_string(),
                password: "p4ssword_long_enough".to_string(),
                membership_type_id: None,
                ..Default::default()
            })
            .await
            .unwrap();

        let event_repo: Arc<dyn EventRepository> =
            Arc::new(SqliteEventRepository::new(pool.clone()));
        let now = Utc::now();
        let event_id = Uuid::new_v4();
        event_repo
            .create(Event {
                id: event_id,
                title: "Upcoming Meetup".to_string(),
                description: "Body".to_string(),
                event_type: EventType::Social,
                event_type_id: None,
                visibility: EventVisibility::Public,
                start_time: now + chrono::Duration::days(7),
                end_time: None,
                timezone: "UTC".to_string(),
                location: None,
                max_attendees: None,
                rsvp_required: false,
                member_price_cents: 0,
                image_url: None,
                created_by: member.id,
                created_at: now,
                updated_at: now,
                series_id: None,
                occurrence_index: None,
            })
            .await
            .unwrap();

        // Member registered elsewhere; now they load the dashboard.
        event_repo
            .register_attendance(event_id, member.id)
            .await
            .unwrap();

        let app = Router::new()
            .route("/portal/api/events/upcoming", get(upcoming_events))
            .layer(Extension(CurrentUser { member }))
            .with_state(event_repo);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/portal/api/events/upcoming")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();

        assert!(
            body.contains("Cancel RSVP"),
            "attending fragment must offer a working Cancel RSVP control, got: {body}"
        );
        assert!(
            body.contains(r#"hx-target="closest div.text-right""#),
            "cancel control must target div.text-right so the toggle survives swaps, got: {body}"
        );
        assert!(
            !body.contains("Attending</span>"),
            "attending state must not be a dead <span>Attending</span>, got: {body}"
        );
    }
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
    State(payment_repo): State<Arc<dyn PaymentRepository>>,
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {
    // Most-recent five payments for this member, regardless of status.
    let mut payments = payment_repo
        .find_by_member(current_user.member.id)
        .await
        .unwrap_or_default();
    payments.truncate(5);

    // Transform to our summary format
    let recent_payments: Vec<PaymentSummary> = payments
        .into_iter()
        .map(|p| PaymentSummary {
            id: p.id.to_string(),
            amount: format!("${:.2}", p.amount_cents as f64 / 100.0),
            status: format!("{:?}", p.status),
            // paid_at is when the money moved (correct for imported/backdated
            // rows); created_at is just row insertion. Same convention as the
            // payments-page list and receipts.
            date: p
                .paid_at
                .unwrap_or(p.created_at)
                .format("%B %d, %Y")
                .to_string(),
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
                payment.description, payment.date, payment.amount, status_class, payment.status
            ));
        }
        html.push_str("</div>");
        html
    };

    axum::response::Html(html)
}
