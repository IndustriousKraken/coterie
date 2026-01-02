mod admin;
mod dashboard;
mod events;
mod payments;
mod profile;

use axum::{
    Router,
    routing::{get, post},
    middleware,
};
use crate::api::state::AppState;

pub fn create_portal_routes(state: AppState) -> Router<AppState> {
    Router::new()
        // Member routes
        .route("/dashboard", get(dashboard::member_dashboard))
        .route("/events", get(events::events_page))
        .route("/payments", get(payments::payments_page))
        .route("/profile", get(profile::profile_page))
        .route("/profile", post(profile::update_profile))
        .route("/profile/password", post(profile::update_password))

        // API endpoints for dashboard
        .route("/api/events/upcoming", get(dashboard::upcoming_events))
        .route("/api/events/list", get(events::events_list_api))
        .route("/api/payments/recent", get(dashboard::recent_payments))
        .route("/api/payments/list", get(payments::payments_list_api))
        .route("/api/payments/summary", get(payments::payments_summary_api))
        .route("/api/payments/dues-status", get(payments::dues_status_api))
        .route("/api/payments/next-due", get(payments::next_due_api))

        // Admin routes
        .route("/admin", get(|| async { "Admin dashboard (TODO)" }))
        .route("/admin/members", get(admin::members::admin_members_page))
        .route("/admin/members/new", get(admin::members::admin_new_member_page))
        .route("/admin/members/new", post(admin::members::admin_create_member))
        .route("/admin/members/:id", get(admin::members::admin_member_detail_page))
        .route("/admin/members/:id/update", post(admin::members::admin_update_member))
        .route("/admin/members/:id/activate", post(admin::members::admin_activate_member))
        .route("/admin/members/:id/suspend", post(admin::members::admin_suspend_member))
        .route("/admin/members/:id/extend-dues", post(admin::members::admin_extend_dues))
        .route("/admin/members/:id/set-dues", post(admin::members::admin_set_dues))
        .route("/admin/members/:id/expire-now", post(admin::members::admin_expire_now))
        .route("/admin/members/:id/payments", get(admin::members::admin_member_payments))
        // Admin event routes
        .route("/admin/events", get(admin::events::admin_events_page))
        .route("/admin/events/new", get(admin::events::admin_new_event_page))
        .route("/admin/events/new", post(admin::events::admin_create_event))
        .route("/admin/events/:id", get(admin::events::admin_event_detail_page))
        .route("/admin/events/:id/update", post(admin::events::admin_update_event))
        .route("/admin/events/:id/delete", post(admin::events::admin_delete_event))
        // Admin announcement routes
        .route("/admin/announcements", get(admin::announcements::admin_announcements_page))
        .route("/admin/announcements/new", get(admin::announcements::admin_new_announcement_page))
        .route("/admin/announcements/new", post(admin::announcements::admin_create_announcement))
        .route("/admin/announcements/:id", get(admin::announcements::admin_announcement_detail_page))
        .route("/admin/announcements/:id/update", post(admin::announcements::admin_update_announcement))
        .route("/admin/announcements/:id/delete", post(admin::announcements::admin_delete_announcement))
        .route("/admin/announcements/:id/publish", post(admin::announcements::admin_publish_announcement))
        .route("/admin/announcements/:id/unpublish", post(admin::announcements::admin_unpublish_announcement))
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

// Shared types used across portal modules
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

// Helper function to check if a user is an admin
pub fn is_admin(member: &crate::domain::Member) -> bool {
    member.notes
        .as_ref()
        .map(|n| n.contains("ADMIN"))
        .unwrap_or(false)
}
