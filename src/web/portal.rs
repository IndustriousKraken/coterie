use axum::{
    Router,
    routing::{get, post},
    middleware,
};
use crate::api::state::AppState;

pub fn create_portal_routes(state: AppState) -> Router<AppState> {
    Router::new()
        // Member routes
        .route("/dashboard", get(member_dashboard))
        .route("/events", get(events_page))
        .route("/payments", get(payments_page))
        .route("/profile", get(profile_page))
        .route("/profile", post(update_profile))
        .route("/profile/password", post(update_password))

        // API endpoints for dashboard
        .route("/api/events/upcoming", get(upcoming_events))
        .route("/api/events/list", get(events_list_api))
        .route("/api/payments/recent", get(recent_payments))
        .route("/api/payments/list", get(payments_list_api))
        .route("/api/payments/summary", get(payments_summary_api))
        .route("/api/payments/dues-status", get(dues_status_api))
        .route("/api/payments/next-due", get(next_due_api))

        // Admin routes
        .route("/admin", get(|| async { "Admin dashboard (TODO)" }))
        .route("/admin/members", get(admin_members_page))
        .route("/admin/members/new", get(admin_new_member_page))
        .route("/admin/members/new", post(admin_create_member))
        .route("/admin/members/:id", get(admin_member_detail_page))
        .route("/admin/members/:id/update", post(admin_update_member))
        .route("/admin/members/:id/activate", post(admin_activate_member))
        .route("/admin/members/:id/suspend", post(admin_suspend_member))
        .route("/admin/members/:id/extend-dues", post(admin_extend_dues))
        .route("/admin/members/:id/set-dues", post(admin_set_dues))
        .route("/admin/members/:id/expire-now", post(admin_expire_now))
        .route("/admin/members/:id/payments", get(admin_member_payments))
        .route("/admin/settings", get(|| async { "Admin settings page (TODO)" }))

        // CSRF protection for state-changing requests (runs after auth)
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            crate::api::middleware::auth::require_csrf,
        ))
        // Require authentication for all portal routes (runs first)
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
use serde::{Deserialize, Serialize};

use crate::{
    api::{
        middleware::auth::{CurrentUser, SessionInfo},
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
    
    // Check if user is admin (notes field contains "ADMIN")
    let is_admin = current_user.member.notes
        .as_ref()
        .map(|n| n.contains("ADMIN"))
        .unwrap_or(false);
    
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

// Profile page
#[derive(Template)]
#[template(path = "portal/profile.html")]
pub struct ProfileTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub member: MemberInfo,
    pub csrf_token: String,
}

async fn profile_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
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

    let is_admin = current_user.member.notes
        .as_ref()
        .map(|n| n.contains("ADMIN"))
        .unwrap_or(false);

    // Generate CSRF token for this session
    let csrf_token = state.service_context.csrf_service
        .generate_token(&session_info.session_id)
        .await
        .unwrap_or_else(|_| "error".to_string());

    let template = ProfileTemplate {
        current_user: Some(user_info),
        is_admin,
        member: member_info,
        csrf_token,
    };

    HtmlTemplate(template)
}

#[derive(Debug, Deserialize)]
pub struct UpdateProfileRequest {
    pub full_name: String,
    #[allow(dead_code)]
    pub csrf_token: String,
}

async fn update_profile(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    axum::Form(form): axum::Form<UpdateProfileRequest>,
) -> axum::response::Response {
    use crate::domain::UpdateMemberRequest;

    let update = UpdateMemberRequest {
        full_name: Some(form.full_name.clone()),
        ..Default::default()
    };

    match state.service_context.member_repo.update(current_user.member.id, update).await {
        Ok(_) => {
            // Redirect back to profile with success message
            axum::response::Response::builder()
                .status(200)
                .header("HX-Redirect", "/portal/profile")
                .header("X-Toast", r#"{"message":"Profile updated successfully!","type":"success"}"#)
                .body(axum::body::Body::empty())
                .unwrap()
        }
        Err(e) => {
            let html = format!(
                "<div class=\"p-4 bg-red-50 text-red-800 rounded-md\">Failed to update profile: {}</div>",
                e
            );
            axum::response::Html(html).into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdatePasswordRequest {
    pub current_password: String,
    pub new_password: String,
    pub confirm_password: String,
    #[allow(dead_code)]
    pub csrf_token: String,
}

async fn update_password(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    axum::Form(form): axum::Form<UpdatePasswordRequest>,
) -> impl IntoResponse {
    // Validate passwords match
    if form.new_password != form.confirm_password {
        return axum::response::Html(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">
                New passwords do not match
            </div>"#.to_string()
        );
    }

    // Validate password length
    if form.new_password.len() < 8 {
        return axum::response::Html(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">
                Password must be at least 8 characters
            </div>"#.to_string()
        );
    }

    // Verify current password
    let password_hash = crate::auth::get_password_hash(
        &state.service_context.db_pool,
        &current_user.member.email
    ).await.ok().flatten();

    let password_valid = if let Some(hash) = password_hash {
        crate::auth::AuthService::verify_password(&form.current_password, &hash)
            .await
            .unwrap_or(false)
    } else {
        false
    };

    if !password_valid {
        return axum::response::Html(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">
                Current password is incorrect
            </div>"#.to_string()
        );
    }

    // Hash new password and update
    use argon2::{Argon2, PasswordHasher};
    use argon2::password_hash::{SaltString, rand_core::OsRng};

    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();

    let new_hash = match argon2.hash_password(form.new_password.as_bytes(), &salt) {
        Ok(hash) => hash.to_string(),
        Err(_) => {
            return axum::response::Html(
                r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">
                    Failed to update password
                </div>"#.to_string()
            );
        }
    };

    // Update password in database
    let result = sqlx::query("UPDATE members SET password_hash = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(&new_hash)
        .bind(current_user.member.id.to_string())
        .execute(&state.service_context.db_pool)
        .await;

    match result {
        Ok(_) => axum::response::Html(
            r#"<div class="p-3 bg-green-50 text-green-800 rounded-md text-sm">
                Password updated successfully!
            </div>"#.to_string()
        ),
        Err(_) => axum::response::Html(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">
                Failed to update password
            </div>"#.to_string()
        ),
    }
}

// Events page
#[derive(Template)]
#[template(path = "portal/events.html")]
pub struct EventsTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
}

async fn events_page(
    State(_state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {
    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let is_admin = current_user.member.notes
        .as_ref()
        .map(|n| n.contains("ADMIN"))
        .unwrap_or(false);

    let template = EventsTemplate {
        current_user: Some(user_info),
        is_admin,
    };

    HtmlTemplate(template)
}

// Payments page
#[derive(Template)]
#[template(path = "portal/payments.html")]
pub struct PaymentsTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
}

async fn payments_page(
    State(_state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {
    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let is_admin = current_user.member.notes
        .as_ref()
        .map(|n| n.contains("ADMIN"))
        .unwrap_or(false);

    let template = PaymentsTemplate {
        current_user: Some(user_info),
        is_admin,
    };

    HtmlTemplate(template)
}

// API endpoint for events list (for events page)
#[derive(Debug, Deserialize)]
pub struct EventsListQuery {
    pub event_type: Option<String>,
    pub show_past: Option<bool>,
}

async fn events_list_api(
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

// API endpoint for full payments list
async fn payments_list_api(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {
    let payments = state.service_context.payment_repo
        .find_by_member(current_user.member.id)
        .await
        .unwrap_or_default();

    if payments.is_empty() {
        return axum::response::Html(
            r#"<div class="p-6 text-center text-gray-500">
                No payment history
            </div>"#.to_string()
        );
    }

    let mut html = String::from(r#"<div class="divide-y">"#);

    for payment in payments {
        let status_badge = match format!("{:?}", payment.status).as_str() {
            "Completed" => r#"<span class="px-2 py-1 text-xs font-medium rounded bg-green-100 text-green-800">Completed</span>"#,
            "Pending" => r#"<span class="px-2 py-1 text-xs font-medium rounded bg-yellow-100 text-yellow-800">Pending</span>"#,
            "Failed" => r#"<span class="px-2 py-1 text-xs font-medium rounded bg-red-100 text-red-800">Failed</span>"#,
            "Refunded" => r#"<span class="px-2 py-1 text-xs font-medium rounded bg-gray-100 text-gray-800">Refunded</span>"#,
            _ => "",
        };

        let description = if payment.description.is_empty() {
            "Membership dues".to_string()
        } else {
            payment.description.clone()
        };

        html.push_str(&format!(
            r#"<div class="px-6 py-4 flex justify-between items-center">
                <div>
                    <p class="font-medium text-gray-900">{}</p>
                    <p class="text-sm text-gray-500">{}</p>
                </div>
                <div class="text-right">
                    <p class="font-medium text-gray-900">${:.2}</p>
                    <div class="mt-1">{}</div>
                </div>
            </div>"#,
            description,
            payment.created_at.format("%B %d, %Y"),
            payment.amount_cents as f64 / 100.0,
            status_badge
        ));
    }

    html.push_str("</div>");
    axum::response::Html(html)
}

// API endpoint for payments summary
async fn payments_summary_api(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {
    use crate::domain::PaymentStatus;

    let payments = state.service_context.payment_repo
        .find_by_member(current_user.member.id)
        .await
        .unwrap_or_default();

    let total: i64 = payments.iter()
        .filter(|p| p.status == PaymentStatus::Completed)
        .map(|p| p.amount_cents)
        .sum();

    axum::response::Html(format!("${:.2}", total as f64 / 100.0))
}

// API endpoint for dues status
async fn dues_status_api(
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {
    let status = if let Some(dues_until) = current_user.member.dues_paid_until {
        if dues_until > chrono::Utc::now() {
            r#"<span class="text-green-600">Current</span>"#
        } else {
            r#"<span class="text-red-600">Expired</span>"#
        }
    } else {
        r#"<span class="text-yellow-600">Unpaid</span>"#
    };

    axum::response::Html(status.to_string())
}

// API endpoint for next due date
async fn next_due_api(
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {
    let next_due = if let Some(dues_until) = current_user.member.dues_paid_until {
        dues_until.format("%B %d, %Y").to_string()
    } else {
        "—".to_string()
    };

    axum::response::Html(next_due)
}

// ============================================================================
// Admin Handlers
// ============================================================================

#[derive(Template)]
#[template(path = "admin/members.html")]
pub struct AdminMembersTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub csrf_token: String,
    pub members: Vec<AdminMemberInfo>,
    pub total_members: i64,
    pub current_page: i64,
    pub per_page: i64,
    pub total_pages: i64,
    pub search_query: String,
    pub status_filter: String,
    pub type_filter: String,
    pub sort_field: String,
    pub sort_order: String,
}

#[derive(Clone)]
pub struct AdminMemberInfo {
    pub id: String,
    pub email: String,
    pub username: String,
    pub full_name: String,
    pub initials: String,
    pub status: String,
    pub membership_type: String,
    pub joined_at: String,
    pub dues_paid_until: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AdminMembersQuery {
    pub q: Option<String>,
    pub status: Option<String>,
    #[serde(rename = "type")]
    pub member_type: Option<String>,
    pub page: Option<i64>,
    pub sort: Option<String>,  // name, status, type, joined, dues
    pub order: Option<String>, // asc, desc
}

async fn admin_members_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Query(query): Query<AdminMembersQuery>,
) -> impl IntoResponse {
    use crate::repository::MemberRepository;

    // Check admin access
    let is_admin = current_user.member.notes
        .as_ref()
        .map(|n| n.contains("ADMIN"))
        .unwrap_or(false);

    if !is_admin {
        return HtmlTemplate(AdminMembersTemplate {
            current_user: None,
            is_admin: false,
            csrf_token: String::new(),
            members: vec![],
            total_members: 0,
            current_page: 1,
            per_page: 20,
            total_pages: 0,
            search_query: String::new(),
            status_filter: String::new(),
            type_filter: String::new(),
            sort_field: "name".to_string(),
            sort_order: "asc".to_string(),
        });
    }

    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    // Generate CSRF token
    let csrf_token = state.service_context.csrf_service
        .generate_token(&session_info.session_id)
        .await
        .unwrap_or_else(|_| "error".to_string());

    // Pagination
    let page = query.page.unwrap_or(1).max(1);
    let per_page: i64 = 20;
    let offset = (page - 1) * per_page;

    // Get all members (we'll filter in memory for now - could optimize with SQL later)
    let all_members = state.service_context.member_repo
        .list(1000, 0)
        .await
        .unwrap_or_default();

    let search_query = query.q.clone().unwrap_or_default().to_lowercase();
    let status_filter = query.status.clone().unwrap_or_default();
    let type_filter = query.member_type.clone().unwrap_or_default();

    // Sort parameters
    let sort_field = query.sort.clone().unwrap_or_else(|| "name".to_string());
    let sort_order = query.order.clone().unwrap_or_else(|| "asc".to_string());

    // Filter members
    let mut filtered_members: Vec<_> = all_members.into_iter()
        .filter(|m| {
            // Search filter
            if !search_query.is_empty() {
                let matches = m.full_name.to_lowercase().contains(&search_query)
                    || m.email.to_lowercase().contains(&search_query)
                    || m.username.to_lowercase().contains(&search_query);
                if !matches {
                    return false;
                }
            }
            // Status filter
            if !status_filter.is_empty() {
                if format!("{:?}", m.status) != status_filter {
                    return false;
                }
            }
            // Type filter
            if !type_filter.is_empty() {
                if format!("{:?}", m.membership_type) != type_filter {
                    return false;
                }
            }
            true
        })
        .collect();

    // Sort members
    filtered_members.sort_by(|a, b| {
        let cmp = match sort_field.as_str() {
            "name" => {
                // Sort by last name, then first name
                let a_parts: Vec<&str> = a.full_name.split_whitespace().collect();
                let b_parts: Vec<&str> = b.full_name.split_whitespace().collect();
                let a_last = a_parts.last().unwrap_or(&"");
                let b_last = b_parts.last().unwrap_or(&"");
                a_last.to_lowercase().cmp(&b_last.to_lowercase())
                    .then_with(|| a.full_name.to_lowercase().cmp(&b.full_name.to_lowercase()))
            }
            "status" => format!("{:?}", a.status).cmp(&format!("{:?}", b.status)),
            "type" => format!("{:?}", a.membership_type).cmp(&format!("{:?}", b.membership_type)),
            "joined" => a.joined_at.cmp(&b.joined_at),
            "dues" => {
                // None values sort last
                match (&a.dues_paid_until, &b.dues_paid_until) {
                    (Some(a_date), Some(b_date)) => a_date.cmp(b_date),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                }
            }
            _ => a.full_name.to_lowercase().cmp(&b.full_name.to_lowercase()),
        };
        if sort_order == "desc" { cmp.reverse() } else { cmp }
    });

    let total_members = filtered_members.len() as i64;
    let total_pages = (total_members + per_page - 1) / per_page;

    // Paginate
    let paginated_members: Vec<AdminMemberInfo> = filtered_members
        .into_iter()
        .skip(offset as usize)
        .take(per_page as usize)
        .map(|m| {
            let initials: String = m.full_name
                .split_whitespace()
                .filter_map(|word| word.chars().next())
                .take(2)
                .collect::<String>()
                .to_uppercase();

            AdminMemberInfo {
                id: m.id.to_string(),
                email: m.email,
                username: m.username,
                full_name: m.full_name,
                initials: if initials.is_empty() { "?".to_string() } else { initials },
                status: format!("{:?}", m.status),
                membership_type: format!("{:?}", m.membership_type),
                joined_at: m.joined_at.format("%b %d, %Y").to_string(),
                dues_paid_until: m.dues_paid_until.map(|d| d.format("%b %d, %Y").to_string()),
            }
        })
        .collect();

    let template = AdminMembersTemplate {
        current_user: Some(user_info),
        is_admin,
        csrf_token,
        members: paginated_members,
        total_members,
        current_page: page,
        per_page,
        total_pages,
        search_query: query.q.unwrap_or_default(),
        status_filter: query.status.unwrap_or_default(),
        type_filter: query.member_type.unwrap_or_default(),
        sort_field,
        sort_order,
    };

    HtmlTemplate(template)
}

#[derive(Debug, Deserialize)]
pub struct MemberIdPath {
    pub id: String,
}

async fn admin_activate_member(
    State(state): State<AppState>,
    Extension(_current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
) -> impl IntoResponse {
    use crate::repository::MemberRepository;
    use crate::domain::{UpdateMemberRequest, MemberStatus};

    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html("<tr><td colspan='6' class='px-6 py-4 text-red-600'>Invalid member ID</td></tr>".to_string()),
    };

    let update = UpdateMemberRequest {
        status: Some(MemberStatus::Active),
        ..Default::default()
    };

    match state.service_context.member_repo.update(id, update).await {
        Ok(member) => {
            let initials: String = member.full_name
                .split_whitespace()
                .filter_map(|word| word.chars().next())
                .take(2)
                .collect::<String>()
                .to_uppercase();

            axum::response::Html(format!(
                r#"<tr class="hover:bg-gray-50 bg-green-50" x-data="{{ open: false }}">
                    <td class="px-6 py-4 whitespace-nowrap">
                        <div class="flex items-center">
                            <div class="flex-shrink-0 h-10 w-10 bg-gray-200 rounded-full flex items-center justify-center">
                                <span class="text-gray-600 font-medium text-sm">{}</span>
                            </div>
                            <div class="ml-4">
                                <div class="text-sm font-medium text-gray-900">{}</div>
                                <div class="text-sm text-gray-500">{}</div>
                                <div class="text-xs text-gray-400">@{}</div>
                            </div>
                        </div>
                    </td>
                    <td class="px-6 py-4 whitespace-nowrap">
                        <span class="px-2 inline-flex text-xs leading-5 font-semibold rounded-full bg-green-100 text-green-800">Active</span>
                    </td>
                    <td class="px-6 py-4 whitespace-nowrap text-sm text-gray-500">{:?}</td>
                    <td class="px-6 py-4 whitespace-nowrap text-sm text-gray-500">{}</td>
                    <td class="px-6 py-4 whitespace-nowrap text-sm text-gray-500">{}</td>
                    <td class="px-6 py-4 whitespace-nowrap text-right text-sm font-medium">
                        <span class="text-green-600">Activated!</span>
                    </td>
                </tr>"#,
                initials,
                member.full_name,
                member.email,
                member.username,
                member.membership_type,
                member.joined_at.format("%b %d, %Y"),
                member.dues_paid_until.map(|d| d.format("%b %d, %Y").to_string()).unwrap_or_else(|| "—".to_string())
            ))
        }
        Err(e) => {
            axum::response::Html(format!(
                "<tr><td colspan='6' class='px-6 py-4 text-red-600'>Error: {}</td></tr>",
                e
            ))
        }
    }
}

async fn admin_suspend_member(
    State(state): State<AppState>,
    Extension(_current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
) -> impl IntoResponse {
    use crate::repository::MemberRepository;
    use crate::domain::{UpdateMemberRequest, MemberStatus};

    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html("<tr><td colspan='6' class='px-6 py-4 text-red-600'>Invalid member ID</td></tr>".to_string()),
    };

    let update = UpdateMemberRequest {
        status: Some(MemberStatus::Suspended),
        ..Default::default()
    };

    match state.service_context.member_repo.update(id, update).await {
        Ok(member) => {
            let initials: String = member.full_name
                .split_whitespace()
                .filter_map(|word| word.chars().next())
                .take(2)
                .collect::<String>()
                .to_uppercase();

            axum::response::Html(format!(
                r#"<tr class="hover:bg-gray-50 bg-yellow-50" x-data="{{ open: false }}">
                    <td class="px-6 py-4 whitespace-nowrap">
                        <div class="flex items-center">
                            <div class="flex-shrink-0 h-10 w-10 bg-gray-200 rounded-full flex items-center justify-center">
                                <span class="text-gray-600 font-medium text-sm">{}</span>
                            </div>
                            <div class="ml-4">
                                <div class="text-sm font-medium text-gray-900">{}</div>
                                <div class="text-sm text-gray-500">{}</div>
                                <div class="text-xs text-gray-400">@{}</div>
                            </div>
                        </div>
                    </td>
                    <td class="px-6 py-4 whitespace-nowrap">
                        <span class="px-2 inline-flex text-xs leading-5 font-semibold rounded-full bg-gray-100 text-gray-800">Suspended</span>
                    </td>
                    <td class="px-6 py-4 whitespace-nowrap text-sm text-gray-500">{:?}</td>
                    <td class="px-6 py-4 whitespace-nowrap text-sm text-gray-500">{}</td>
                    <td class="px-6 py-4 whitespace-nowrap text-sm text-gray-500">{}</td>
                    <td class="px-6 py-4 whitespace-nowrap text-right text-sm font-medium">
                        <span class="text-yellow-600">Suspended</span>
                    </td>
                </tr>"#,
                initials,
                member.full_name,
                member.email,
                member.username,
                member.membership_type,
                member.joined_at.format("%b %d, %Y"),
                member.dues_paid_until.map(|d| d.format("%b %d, %Y").to_string()).unwrap_or_else(|| "—".to_string())
            ))
        }
        Err(e) => {
            axum::response::Html(format!(
                "<tr><td colspan='6' class='px-6 py-4 text-red-600'>Error: {}</td></tr>",
                e
            ))
        }
    }
}

// ============================================================================
// Admin Member Detail Handlers
// ============================================================================

#[derive(Template)]
#[template(path = "admin/member_detail.html")]
pub struct AdminMemberDetailTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub csrf_token: String,
    pub member: AdminMemberDetailInfo,
}

pub struct AdminMemberDetailInfo {
    pub id: String,
    pub email: String,
    pub username: String,
    pub full_name: String,
    pub initials: String,
    pub status: String,
    pub membership_type: String,
    pub joined_at: String,
    pub dues_paid_until: Option<String>,
    pub dues_expired: bool,
    pub bypass_dues: bool,
    pub notes: String,
    pub created_at: String,
    pub updated_at: String,
}

async fn admin_member_detail_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Path(member_id): Path<String>,
) -> axum::response::Response {
    use crate::repository::MemberRepository;

    // Check admin access
    let is_admin = current_user.member.notes
        .as_ref()
        .map(|n| n.contains("ADMIN"))
        .unwrap_or(false);

    if !is_admin {
        return axum::response::Redirect::to("/portal/dashboard").into_response();
    }

    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Redirect::to("/portal/admin/members").into_response(),
    };

    let member = match state.service_context.member_repo.find_by_id(id).await {
        Ok(Some(m)) => m,
        _ => return axum::response::Redirect::to("/portal/admin/members").into_response(),
    };

    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let csrf_token = state.service_context.csrf_service
        .generate_token(&session_info.session_id)
        .await
        .unwrap_or_else(|_| "error".to_string());

    let initials: String = member.full_name
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase();

    let now = chrono::Utc::now();
    let dues_expired = member.dues_paid_until
        .map(|d| d < now)
        .unwrap_or(true);

    let member_info = AdminMemberDetailInfo {
        id: member.id.to_string(),
        email: member.email,
        username: member.username,
        full_name: member.full_name,
        initials: if initials.is_empty() { "?".to_string() } else { initials },
        status: format!("{:?}", member.status),
        membership_type: format!("{:?}", member.membership_type),
        joined_at: member.joined_at.format("%B %d, %Y").to_string(),
        dues_paid_until: member.dues_paid_until.map(|d| d.format("%B %d, %Y").to_string()),
        dues_expired,
        bypass_dues: member.bypass_dues,
        notes: member.notes.unwrap_or_default(),
        created_at: member.created_at.format("%B %d, %Y").to_string(),
        updated_at: member.updated_at.format("%B %d, %Y at %l:%M %p").to_string(),
    };

    let template = AdminMemberDetailTemplate {
        current_user: Some(user_info),
        is_admin,
        csrf_token,
        member: member_info,
    };

    HtmlTemplate(template).into_response()
}

#[derive(Debug, Deserialize)]
pub struct AdminUpdateMemberForm {
    pub full_name: String,
    pub membership_type: String,
    pub notes: Option<String>,
    pub bypass_dues: Option<String>,
    #[allow(dead_code)]
    pub csrf_token: String,
}

async fn admin_update_member(
    State(state): State<AppState>,
    Extension(_current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
    axum::Form(form): axum::Form<AdminUpdateMemberForm>,
) -> impl IntoResponse {
    use crate::repository::MemberRepository;
    use crate::domain::{UpdateMemberRequest, MembershipType};

    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">Invalid member ID</div>"#.to_string()
        ),
    };

    let membership_type = match form.membership_type.as_str() {
        "Regular" => MembershipType::Regular,
        "Student" => MembershipType::Student,
        "Corporate" => MembershipType::Corporate,
        "Lifetime" => MembershipType::Lifetime,
        _ => MembershipType::Regular,
    };

    let update = UpdateMemberRequest {
        full_name: Some(form.full_name),
        membership_type: Some(membership_type),
        notes: Some(form.notes.unwrap_or_default()),
        bypass_dues: Some(form.bypass_dues.is_some()),
        ..Default::default()
    };

    match state.service_context.member_repo.update(id, update).await {
        Ok(_) => axum::response::Html(
            r#"<div class="p-3 bg-green-50 text-green-800 rounded-md text-sm">Member updated successfully!</div>"#.to_string()
        ),
        Err(e) => axum::response::Html(format!(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">Error: {}</div>"#,
            e
        )),
    }
}

#[derive(Debug, Deserialize)]
pub struct ExtendDuesForm {
    pub months: i32,
    #[allow(dead_code)]
    pub csrf_token: String,
}

async fn admin_extend_dues(
    State(state): State<AppState>,
    Extension(_current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
    axum::Form(form): axum::Form<ExtendDuesForm>,
) -> impl IntoResponse {
    use crate::repository::MemberRepository;
    use chrono::Months;

    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">Invalid member ID</div>"#.to_string()
        ),
    };

    // Get current member
    let member = match state.service_context.member_repo.find_by_id(id).await {
        Ok(Some(m)) => m,
        _ => return axum::response::Html(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">Member not found</div>"#.to_string()
        ),
    };

    // Calculate new dues date
    let now = chrono::Utc::now();
    let base_date = member.dues_paid_until
        .filter(|d| *d > now)
        .unwrap_or(now);

    let new_dues_date = base_date
        .checked_add_months(Months::new(form.months as u32))
        .unwrap_or(base_date);

    // Update in database
    let result = sqlx::query("UPDATE members SET dues_paid_until = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(new_dues_date)
        .bind(member_id)
        .execute(&state.service_context.db_pool)
        .await;

    match result {
        Ok(_) => axum::response::Html(format!(
            r#"<div class="p-3 bg-green-50 text-green-800 rounded-md text-sm">
                Dues extended! New expiration: {}
                <script>setTimeout(() => location.reload(), 1500)</script>
            </div>"#,
            new_dues_date.format("%B %d, %Y")
        )),
        Err(e) => axum::response::Html(format!(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">Error: {}</div>"#,
            e
        )),
    }
}

#[derive(Debug, Deserialize)]
pub struct SetDuesForm {
    pub dues_until: String,
    #[allow(dead_code)]
    pub csrf_token: String,
}

async fn admin_set_dues(
    State(state): State<AppState>,
    Extension(_current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
    axum::Form(form): axum::Form<SetDuesForm>,
) -> impl IntoResponse {
    use chrono::NaiveDate;

    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">Invalid member ID</div>"#.to_string()
        ),
    };

    // Parse the date
    let naive_date = match NaiveDate::parse_from_str(&form.dues_until, "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => return axum::response::Html(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">Invalid date format</div>"#.to_string()
        ),
    };

    // Convert to DateTime<Utc> at end of day
    let dues_date = naive_date
        .and_hms_opt(23, 59, 59)
        .unwrap()
        .and_utc();

    // Update in database
    let result = sqlx::query("UPDATE members SET dues_paid_until = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(dues_date)
        .bind(id.to_string())
        .execute(&state.service_context.db_pool)
        .await;

    match result {
        Ok(_) => axum::response::Html(format!(
            r#"<div class="p-3 bg-green-50 text-green-800 rounded-md text-sm">
                Dues date set to: {}
                <script>setTimeout(() => location.reload(), 1500)</script>
            </div>"#,
            dues_date.format("%B %d, %Y")
        )),
        Err(e) => axum::response::Html(format!(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">Error: {}</div>"#,
            e
        )),
    }
}

async fn admin_expire_now(
    State(state): State<AppState>,
    Extension(_current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
) -> impl IntoResponse {
    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">Invalid member ID</div>"#.to_string()
        ),
    };

    // Set dues to yesterday
    let yesterday = chrono::Utc::now() - chrono::Duration::days(1);

    let result = sqlx::query("UPDATE members SET dues_paid_until = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(yesterday)
        .bind(id.to_string())
        .execute(&state.service_context.db_pool)
        .await;

    match result {
        Ok(_) => axum::response::Html(
            r#"<div class="p-3 bg-yellow-50 text-yellow-800 rounded-md text-sm">
                Member dues have been expired.
                <script>setTimeout(() => location.reload(), 1500)</script>
            </div>"#.to_string()
        ),
        Err(e) => axum::response::Html(format!(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">Error: {}</div>"#,
            e
        )),
    }
}

async fn admin_member_payments(
    State(state): State<AppState>,
    Extension(_current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
) -> impl IntoResponse {
    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html(
            r#"<div class="p-6 text-center text-red-600">Invalid member ID</div>"#.to_string()
        ),
    };

    let payments = state.service_context.payment_repo
        .find_by_member(id)
        .await
        .unwrap_or_default();

    if payments.is_empty() {
        return axum::response::Html(
            r#"<div class="p-6 text-center text-gray-500">No payment history for this member</div>"#.to_string()
        );
    }

    let mut html = String::new();

    for payment in payments {
        let status_badge = match format!("{:?}", payment.status).as_str() {
            "Completed" => r#"<span class="px-2 py-1 text-xs font-medium rounded bg-green-100 text-green-800">Completed</span>"#,
            "Pending" => r#"<span class="px-2 py-1 text-xs font-medium rounded bg-yellow-100 text-yellow-800">Pending</span>"#,
            "Failed" => r#"<span class="px-2 py-1 text-xs font-medium rounded bg-red-100 text-red-800">Failed</span>"#,
            "Refunded" => r#"<span class="px-2 py-1 text-xs font-medium rounded bg-gray-100 text-gray-800">Refunded</span>"#,
            _ => "",
        };

        let description = if payment.description.is_empty() {
            "Membership dues".to_string()
        } else {
            payment.description.clone()
        };

        html.push_str(&format!(
            r#"<div class="px-6 py-4 flex justify-between items-center">
                <div>
                    <p class="font-medium text-gray-900">{}</p>
                    <p class="text-sm text-gray-500">{}</p>
                </div>
                <div class="text-right">
                    <p class="font-medium text-gray-900">${:.2}</p>
                    <div class="mt-1">{}</div>
                </div>
            </div>"#,
            description,
            payment.created_at.format("%B %d, %Y"),
            payment.amount_cents as f64 / 100.0,
            status_badge
        ));
    }

    axum::response::Html(html)
}

// ============================================================================
// Admin New Member Handlers
// ============================================================================

#[derive(Template)]
#[template(path = "admin/member_new.html")]
pub struct AdminNewMemberTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub csrf_token: String,
}

async fn admin_new_member_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> axum::response::Response {
    // Check admin access
    let is_admin = current_user.member.notes
        .as_ref()
        .map(|n| n.contains("ADMIN"))
        .unwrap_or(false);

    if !is_admin {
        return axum::response::Redirect::to("/portal/dashboard").into_response();
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

    let template = AdminNewMemberTemplate {
        current_user: Some(user_info),
        is_admin,
        csrf_token,
    };

    HtmlTemplate(template).into_response()
}

#[derive(Debug, Deserialize)]
pub struct AdminCreateMemberForm {
    pub email: String,
    pub username: String,
    pub full_name: String,
    pub password: String,
    pub membership_type: String,
    pub status: String,
    pub notes: Option<String>,
    #[allow(dead_code)]
    pub csrf_token: String,
}

async fn admin_create_member(
    State(state): State<AppState>,
    Extension(_current_user): Extension<CurrentUser>,
    axum::Form(form): axum::Form<AdminCreateMemberForm>,
) -> axum::response::Response {
    use crate::repository::MemberRepository;
    use crate::domain::{CreateMemberRequest, MembershipType, MemberStatus, UpdateMemberRequest};

    let membership_type = match form.membership_type.as_str() {
        "Regular" => MembershipType::Regular,
        "Student" => MembershipType::Student,
        "Corporate" => MembershipType::Corporate,
        "Lifetime" => MembershipType::Lifetime,
        _ => MembershipType::Regular,
    };

    let create_request = CreateMemberRequest {
        email: form.email.clone(),
        username: form.username.clone(),
        full_name: form.full_name.clone(),
        password: form.password,
        membership_type,
    };

    match state.service_context.member_repo.create(create_request).await {
        Ok(member) => {
            // Set status if not pending
            let status = match form.status.as_str() {
                "Active" => Some(MemberStatus::Active),
                "Expired" => Some(MemberStatus::Expired),
                "Suspended" => Some(MemberStatus::Suspended),
                "Honorary" => Some(MemberStatus::Honorary),
                _ => None,
            };

            if status.is_some() || form.notes.is_some() {
                let update = UpdateMemberRequest {
                    status,
                    notes: form.notes,
                    ..Default::default()
                };
                let _ = state.service_context.member_repo.update(member.id, update).await;
            }

            axum::response::Redirect::to(&format!("/portal/admin/members/{}", member.id)).into_response()
        }
        Err(e) => {
            // Return error page
            axum::response::Html(format!(
                r#"<!DOCTYPE html>
                <html>
                <head>
                    <title>Error - Coterie</title>
                    <script src="https://cdn.tailwindcss.com"></script>
                </head>
                <body class="bg-gray-100 min-h-screen flex items-center justify-center">
                    <div class="bg-white p-8 rounded-lg shadow-md max-w-md">
                        <h1 class="text-xl font-bold text-red-600 mb-4">Error Creating Member</h1>
                        <p class="text-gray-700 mb-4">{}</p>
                        <a href="/portal/admin/members/new" class="text-blue-600 hover:underline">Go back and try again</a>
                    </div>
                </body>
                </html>"#,
                e
            )).into_response()
        }
    }
}