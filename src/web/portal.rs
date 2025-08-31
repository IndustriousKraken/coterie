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
    extract::State,
    response::IntoResponse,
    Extension,
};

use crate::{
    api::middleware::auth::CurrentUser,
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