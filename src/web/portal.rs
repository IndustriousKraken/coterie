use axum::{
    Router,
    routing::get,
    middleware,
};
use crate::api::state::AppState;

pub fn create_portal_routes(state: AppState) -> Router<AppState> {
    Router::new()
        // Member routes
        .route("/dashboard", get(member_dashboard))
        .route("/events", get(|| async { "Events page (TODO)" }))
        .route("/payments", get(|| async { "Payments page (TODO)" }))
        .route("/profile", get(|| async { "Profile page (TODO)" }))
        
        // API endpoints for dashboard
        .route("/api/events/upcoming", get(upcoming_events))
        .route("/api/payments/recent", get(recent_payments))
        
        // Admin routes
        .route("/admin", get(|| async { "Admin dashboard (TODO)" }))
        .route("/admin/members", get(|| async { "Admin members page (TODO)" }))
        .route("/admin/settings", get(|| async { "Admin settings page (TODO)" }))
        
        // Require authentication for all portal routes
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            crate::api::middleware::auth::require_auth,
        ))
}

use askama::Template;
use axum::{
    extract::{State, Query, Path},
    response::IntoResponse,
    Extension,
};
use serde::Serialize;

use crate::{
    api::{
        middleware::auth::CurrentUser,
        handlers::{
            events::{ListEventsQuery, list as list_events},
            payments::{ListPaymentsQuery, list_by_member as list_payments},
        },
    },
    web::templates::{HtmlTemplate, UserInfo},
};

#[derive(Template)]
#[template(path = "dashboard/member.html")]
pub struct MemberDashboardTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub member: MemberInfo,
}

pub struct MemberInfo {
    pub id: String,
    pub username: String,
    pub full_name: String,
    pub email: String,
    pub status: String,
    pub membership_type: String,
    pub joined_at: String,
    pub dues_paid_until: Option<String>,
}

async fn member_dashboard(
    State(state): State<AppState>,
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
    
    // TODO: Check if user is admin based on role or membership type
    let is_admin = false;
    
    let template = MemberDashboardTemplate {
        current_user: Some(user_info),
        is_admin,
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
    attending: bool,
}

async fn upcoming_events(
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
    
    // Transform to our summary format
    let event_summaries: Vec<EventSummary> = events.into_iter().map(|event| {
        EventSummary {
            id: event.id.to_string(),
            title: event.title,
            date: event.start_time.format("%B %d, %Y").to_string(),
            time: event.start_time.format("%l:%M %p").to_string(),
            location: event.location,
            attending: false, // TODO: Check attendance when we have a method for it
        }
    }).collect();
    
    // Return HTML fragment for HTMX
    let html = if event_summaries.is_empty() {
        r#"<p class="text-gray-500">No upcoming events</p>"#.to_string()
    } else {
        let mut html = String::from(r#"<div class="space-y-3">"#);
        for event in event_summaries {
            html.push_str(&format!(
                r#"
                <div class="border-l-4 border-blue-500 pl-3">
                    <h3 class="font-medium">{}</h3>
                    <p class="text-sm text-gray-600">{} at {}</p>
                    {}
                    <div class="mt-1">
                        {}
                    </div>
                </div>
                "#,
                event.title,
                event.date,
                event.time,
                event.location.map(|l| format!(r#"<p class="text-sm text-gray-600">📍 {}</p>"#, l)).unwrap_or_default(),
                if event.attending {
                    r#"<span class="text-xs text-green-600">✓ Attending</span>"#.to_string()
                } else {
                    format!(r#"<button hx-post="/portal/events/{}/rsvp" hx-swap="outerHTML" class="text-xs text-blue-600 hover:text-blue-800">RSVP</button>"#, event.id)
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

async fn recent_payments(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {
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