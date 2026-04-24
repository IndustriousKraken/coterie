mod admin;
mod announcements;
mod dashboard;
mod donations;
mod events;
mod payments;
mod profile;
mod restore;

use axum::{
    Router,
    routing::{get, post, put, delete},
    middleware,
};
use crate::api::state::AppState;

pub fn create_portal_routes(state: AppState) -> Router<AppState> {
    // Admin routes — gated at the middleware layer by require_admin_redirect.
    // Non-admins hitting these routes are redirected to /portal/dashboard.
    // Note: there's no bare /portal/admin landing page. The admin nav
    // dropdown links directly to /portal/admin/members, /events, etc.
    // If a member ever hits /portal/admin directly, axum returns 404
    // and the user can use the navigation.
    let admin_routes = Router::new()
        .route("/members", get(admin::members::admin_members_page))
        .route("/members/new", get(admin::members::admin_new_member_page))
        .route("/members/new", post(admin::members::admin_create_member))
        .route("/members/:id", get(admin::members::admin_member_detail_page))
        .route("/members/:id/update", post(admin::members::admin_update_member))
        .route("/members/:id/activate", post(admin::members::admin_activate_member))
        .route("/members/:id/suspend", post(admin::members::admin_suspend_member))
        .route("/members/:id/extend-dues", post(admin::members::admin_extend_dues))
        .route("/members/:id/set-dues", post(admin::members::admin_set_dues))
        .route("/members/:id/expire-now", post(admin::members::admin_expire_now))
        .route("/members/:id/payments", get(admin::members::admin_member_payments))
        .route("/members/:id/resend-verification", post(admin::members::admin_resend_verification))
        // Events
        .route("/events", get(admin::events::admin_events_page))
        .route("/events/new", get(admin::events::admin_new_event_page))
        .route("/events/new", post(admin::events::admin_create_event))
        .route("/events/:id", get(admin::events::admin_event_detail_page))
        .route("/events/:id/update", post(admin::events::admin_update_event))
        .route("/events/:id/delete", post(admin::events::admin_delete_event))
        // Announcements
        .route("/announcements", get(admin::announcements::admin_announcements_page))
        .route("/announcements/new", get(admin::announcements::admin_new_announcement_page))
        .route("/announcements/new", post(admin::announcements::admin_create_announcement))
        .route("/announcements/:id", get(admin::announcements::admin_announcement_detail_page))
        .route("/announcements/:id/update", post(admin::announcements::admin_update_announcement))
        .route("/announcements/:id/delete", post(admin::announcements::admin_delete_announcement))
        .route("/announcements/:id/publish", post(admin::announcements::admin_publish_announcement))
        .route("/announcements/:id/unpublish", post(admin::announcements::admin_unpublish_announcement))
        // Type management
        .route("/types", get(admin::types::admin_types_page))
        .route("/types/event/new", get(admin::types::admin_new_event_type_page))
        .route("/types/event/new", post(admin::types::admin_create_event_type))
        .route("/types/event/:id", get(admin::types::admin_edit_event_type_page))
        .route("/types/event/:id", post(admin::types::admin_update_event_type))
        .route("/types/event/:id/delete", post(admin::types::admin_delete_event_type))
        .route("/types/announcement/new", get(admin::types::admin_new_announcement_type_page))
        .route("/types/announcement/new", post(admin::types::admin_create_announcement_type))
        .route("/types/announcement/:id", get(admin::types::admin_edit_announcement_type_page))
        .route("/types/announcement/:id", post(admin::types::admin_update_announcement_type))
        .route("/types/announcement/:id/delete", post(admin::types::admin_delete_announcement_type))
        .route("/types/membership/new", get(admin::types::admin_new_membership_type_page))
        .route("/types/membership/new", post(admin::types::admin_create_membership_type))
        .route("/types/membership/:id", get(admin::types::admin_edit_membership_type_page))
        .route("/types/membership/:id", post(admin::types::admin_update_membership_type))
        .route("/types/membership/:id/delete", post(admin::types::admin_delete_membership_type))
        // Settings
        .route("/settings", get(admin::settings::admin_settings_page))
        .route("/settings", post(admin::settings::admin_update_setting))
        // Email settings (dedicated page with test button)
        .route("/settings/email", get(admin::email::email_settings_page))
        .route("/settings/email", post(admin::email::update_email_settings))
        .route("/settings/email/test", post(admin::email::send_test_email))
        // Audit log viewer + CSV export
        .route("/audit", get(admin::audit::audit_log_page))
        .route("/audit/export", get(admin::audit::audit_log_export))
        // CSRF runs after auth — in axum, the LAST route_layer is applied
        // OUTERMOST and runs FIRST. So add CSRF first, admin middleware
        // second, so admin runs first (setting SessionInfo) then CSRF.
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            crate::api::middleware::auth::require_csrf,
        ))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            crate::api::middleware::auth::require_admin_redirect,
        ));

    // Restoration routes — allow Expired members alongside Active/Honorary.
    // These are the narrow set of routes an Expired member needs to pay
    // their dues and reactivate their account. Nothing else.
    let restorable_routes = Router::new()
        .route("/restore", get(restore::restore_page))
        // Dues-warning banner (loaded on every portal page by base.html)
        .route("/api/dues-warning", get(dashboard::dues_warning))
        // Payment pages
        .route("/payments/new", get(payments::payment_new_page))
        .route("/payments/methods", get(payments::payment_methods_page))
        .route("/payments/success", get(payments::payment_success_page))
        .route("/payments/cancel", get(payments::payment_cancel_page))
        // Payment/card APIs
        .route("/api/payments/checkout", post(payments::checkout_api))
        .route("/api/payments/charge-saved", post(payments::charge_saved_card_api))
        .route("/api/payments/list", get(payments::payments_list_api))
        .route("/api/payments/summary", get(payments::payments_summary_api))
        .route("/api/payments/dues-status", get(payments::dues_status_api))
        .route("/api/payments/next-due", get(payments::next_due_api))
        .route("/api/payments/cards", get(payments::saved_cards_html_api))
        .route("/api/payments/cards/:card_id", delete(payments::delete_card_api))
        .route("/api/payments/cards/:card_id/default", put(payments::set_default_card_api))
        // CSRF first, auth second (see admin_routes comment on ordering).
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            crate::api::middleware::auth::require_csrf,
        ))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            crate::api::middleware::auth::require_restorable,
        ));

    // Active-only routes — standard member pages. Expired members hitting
    // these get bounced to /portal/restore by require_auth_redirect.
    let active_only_routes = Router::new()
        .route("/dashboard", get(dashboard::member_dashboard))
        .route("/events", get(events::events_page))
        .route("/announcements", get(announcements::announcements_page))
        .route("/payments", get(payments::payments_page))
        .route("/donate", get(donations::donate_page))
        .route("/profile", get(profile::profile_page))
        .route("/profile", post(profile::update_profile))
        .route("/profile/password", post(profile::update_password))
        // API endpoints (HTMX fragments) — for Active members only
        .route("/api/events/upcoming", get(dashboard::upcoming_events))
        .route("/api/events/list", get(events::events_list_api))
        .route("/api/events/:id/rsvp", post(events::rsvp_event))
        .route("/api/events/:id/cancel", post(events::cancel_rsvp_event))
        .route("/api/announcements/list", get(announcements::announcements_list_api))
        .route("/api/payments/recent", get(dashboard::recent_payments))
        .route("/api/donate", post(donations::donate_api))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            crate::api::middleware::auth::require_csrf,
        ))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            crate::api::middleware::auth::require_auth_redirect,
        ));

    Router::new()
        .nest("/admin", admin_routes)
        .merge(restorable_routes)
        .merge(active_only_routes)
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

pub fn is_admin(member: &crate::domain::Member) -> bool {
    member.is_admin
}
