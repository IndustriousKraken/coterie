pub mod admin;
mod announcements;
pub mod dashboard;
mod donations;
mod events;
mod partials;
mod payments;
pub mod profile;
mod restore;
pub mod security;

use crate::api::state::AppState;
use axum::{
    middleware,
    routing::{delete, get, post, put},
    Router,
};

pub fn create_portal_routes(state: AppState) -> Router<AppState> {
    // Admin routes — gated at the middleware layer by require_admin_redirect.
    // Non-admins hitting these routes are redirected to /portal/dashboard.
    // Note: there's no bare /portal/admin landing page. The admin nav
    // dropdown links directly to /portal/admin/members, /events, etc.
    // If a member ever hits /portal/admin directly, axum returns 404
    // and the user can use the navigation.
    let admin_routes = Router::new()
        .route("/members", get(admin::members::list::admin_members_page))
        .route("/members/export", get(admin::members::admin_members_export))
        .route(
            "/members/import",
            get(admin::members::admin_members_import_page),
        )
        .route(
            "/members/import",
            post(admin::members::admin_members_import),
        )
        .route(
            "/members/new",
            get(admin::members::create::admin_new_member_page),
        )
        .route(
            "/members/new",
            post(admin::members::create::admin_create_member),
        )
        .route(
            "/members/:id",
            get(admin::members::detail::admin_member_detail_page),
        )
        .route(
            "/members/:id/update",
            post(admin::members::detail::admin_update_member),
        )
        .route(
            "/members/:id/activate",
            post(admin::members::status::admin_activate_member),
        )
        .route(
            "/members/:id/suspend",
            post(admin::members::status::admin_suspend_member),
        )
        .route(
            "/members/:id/extend-dues",
            post(admin::members::dues::admin_extend_dues),
        )
        .route(
            "/members/:id/set-dues",
            post(admin::members::dues::admin_set_dues),
        )
        .route(
            "/members/:id/expire-now",
            post(admin::members::status::admin_expire_now),
        )
        .route(
            "/members/:id/payments",
            get(admin::members::dues::admin_member_payments),
        )
        .route(
            "/members/:id/record-payment",
            get(admin::members::payments::admin_record_payment_page),
        )
        .route(
            "/members/:id/record-payment",
            post(admin::members::payments::admin_record_payment_submit),
        )
        .route(
            "/payments/:id/refund",
            post(admin::payments::admin_refund_payment),
        )
        .route(
            "/members/:id/resend-verification",
            post(admin::members::verification::admin_resend_verification),
        )
        .route(
            "/members/:id/discord-id",
            post(admin::members::discord::admin_update_discord_id),
        )
        // Events
        .route("/events", get(admin::events::admin_events_page))
        .route("/events/new", get(admin::events::admin_new_event_page))
        .route("/events/new", post(admin::events::admin_create_event))
        .route("/events/:id", get(admin::events::admin_event_detail_page))
        .route(
            "/events/:id/update",
            post(admin::events::admin_update_event),
        )
        .route(
            "/events/:id/delete",
            post(admin::events::admin_delete_event),
        )
        // Per-occurrence exception handlers (cancel / override /
        // restore one occurrence within a recurring series).
        .route(
            "/events/series/:id",
            get(admin::events::admin_event_series_detail_page),
        )
        .route(
            "/events/series/:id/occurrences/:index/override",
            get(admin::events::admin_occurrence_override_form),
        )
        .route(
            "/events/series/:id/occurrences/:index/cancel",
            post(admin::events::admin_cancel_event_occurrence),
        )
        .route(
            "/events/series/:id/occurrences/:index/override",
            post(admin::events::admin_override_event_occurrence),
        )
        .route(
            "/events/series/:id/occurrences/:index/restore",
            post(admin::events::admin_restore_event_occurrence),
        )
        // Announcements
        .route(
            "/announcements",
            get(admin::announcements::admin_announcements_page),
        )
        .route(
            "/announcements/new",
            get(admin::announcements::admin_new_announcement_page),
        )
        .route(
            "/announcements/new",
            post(admin::announcements::admin_create_announcement),
        )
        .route(
            "/announcements/:id",
            get(admin::announcements::admin_announcement_detail_page),
        )
        .route(
            "/announcements/:id/update",
            post(admin::announcements::admin_update_announcement),
        )
        .route(
            "/announcements/:id/delete",
            post(admin::announcements::admin_delete_announcement),
        )
        .route(
            "/announcements/:id/publish",
            post(admin::announcements::admin_publish_announcement),
        )
        .route(
            "/announcements/:id/unpublish",
            post(admin::announcements::admin_unpublish_announcement),
        )
        // Type management. Membership-type routes are registered first
        // with static `membership` segments so Axum's static-over-dynamic
        // matching prefers them; event/announcement types share a single
        // handler set parameterized by `:kind` ("event" | "announcement").
        .route("/types", get(admin::types::admin_types_page))
        .route(
            "/types/membership/new",
            get(admin::types::admin_new_membership_type_page),
        )
        .route(
            "/types/membership/new",
            post(admin::types::admin_create_membership_type),
        )
        .route(
            "/types/membership/:id",
            get(admin::types::admin_edit_membership_type_page),
        )
        .route(
            "/types/membership/:id",
            post(admin::types::admin_update_membership_type),
        )
        .route(
            "/types/membership/:id/delete",
            post(admin::types::admin_delete_membership_type),
        )
        .route(
            "/types/:kind/new",
            get(admin::types::admin_new_basic_type_page),
        )
        .route(
            "/types/:kind/new",
            post(admin::types::admin_create_basic_type),
        )
        .route(
            "/types/:kind/:id",
            get(admin::types::admin_edit_basic_type_page),
        )
        .route(
            "/types/:kind/:id",
            post(admin::types::admin_update_basic_type),
        )
        .route(
            "/types/:kind/:id/delete",
            post(admin::types::admin_delete_basic_type),
        )
        // Settings
        .route("/settings", get(admin::settings::admin_settings_page))
        .route("/settings", post(admin::settings::admin_update_setting))
        // Email settings (dedicated page with test button)
        .route("/settings/email", get(admin::email::email_settings_page))
        .route("/settings/email", post(admin::email::update_email_settings))
        .route("/settings/email/test", post(admin::email::send_test_email))
        // Discord settings (dedicated page with test connection button)
        .route(
            "/settings/discord",
            get(admin::discord::discord_settings_page),
        )
        .route(
            "/settings/discord",
            post(admin::discord::update_discord_settings),
        )
        .route(
            "/settings/discord/test",
            post(admin::discord::test_discord_connection),
        )
        .route(
            "/settings/discord/reconcile",
            post(admin::discord::reconcile_roles),
        )
        // Billing settings (Stripe-sub → Coterie-managed bulk migration)
        .route(
            "/settings/billing",
            get(admin::billing::billing_settings_page),
        )
        .route(
            "/settings/billing/migrate-stripe-subs",
            post(admin::billing::bulk_migrate_stripe_subs),
        )
        // Read-only billing dashboard: upcoming charges, recent
        // failures, revenue by month. Actions stay on the per-member
        // page.
        .route(
            "/billing/dashboard",
            get(admin::billing::billing_dashboard_page),
        )
        // Finance — expense ledger CRUD + reconciliation reports.
        //
        // NOTE: the `money_limiter` rate limiter is intentionally NOT
        // applied here. That limiter exists for endpoints that
        // initiate Stripe charges; recording an internal expense or
        // viewing a report doesn't move money in the
        // payment-processor sense. The shared admin gate +
        // require_admin_redirect + CSRF middleware tree already
        // covers these routes.
        .route(
            "/finance/expenses",
            get(admin::finance::expenses::list_page),
        )
        .route(
            "/finance/expenses/new",
            get(admin::finance::expenses::new_page),
        )
        .route("/finance/expenses", post(admin::finance::expenses::create))
        .route(
            "/finance/expenses/:id/edit",
            get(admin::finance::expenses::edit_page),
        )
        .route(
            "/finance/expenses/:id",
            post(admin::finance::expenses::update),
        )
        .route(
            "/finance/expenses/:id/delete",
            post(admin::finance::expenses::delete),
        )
        .route(
            "/finance/categories",
            get(admin::finance::categories::list_page),
        )
        .route(
            "/finance/categories/new",
            get(admin::finance::categories::new_page),
        )
        .route(
            "/finance/categories",
            post(admin::finance::categories::create),
        )
        .route(
            "/finance/categories/:id/edit",
            get(admin::finance::categories::edit_page),
        )
        .route(
            "/finance/categories/:id",
            post(admin::finance::categories::update),
        )
        .route(
            "/finance/categories/:id/delete",
            post(admin::finance::categories::delete),
        )
        .route(
            "/finance/categories/:id/activate",
            post(admin::finance::categories::activate),
        )
        .route(
            "/finance/categories/:id/deactivate",
            post(admin::finance::categories::deactivate),
        )
        .route(
            "/finance/accounts",
            get(admin::finance::accounts::list_page),
        )
        .route(
            "/finance/accounts/new",
            get(admin::finance::accounts::new_page),
        )
        .route("/finance/accounts", post(admin::finance::accounts::create))
        .route(
            "/finance/accounts/:id/edit",
            get(admin::finance::accounts::edit_page),
        )
        .route(
            "/finance/accounts/:id",
            post(admin::finance::accounts::update),
        )
        .route(
            "/finance/accounts/:id/delete",
            post(admin::finance::accounts::delete),
        )
        .route(
            "/finance/accounts/:id/activate",
            post(admin::finance::accounts::activate),
        )
        .route(
            "/finance/accounts/:id/deactivate",
            post(admin::finance::accounts::deactivate),
        )
        .route(
            "/finance/reports/monthly",
            get(admin::finance::reports::monthly_report),
        )
        .route(
            "/finance/reports/annual",
            get(admin::finance::reports::annual_report),
        )
        .route(
            "/finance/reports/tax-prep",
            get(admin::finance::reports::tax_prep_csv),
        )
        // Audit log viewer + CSV export
        .route("/audit", get(admin::audit::audit_log_page))
        .route("/audit/export", get(admin::audit::audit_log_export))
        // CSRF is enforced at the top of the application router (see
        // `middleware::security::csrf_protect_unless_exempt`); only the
        // admin gate is layered here.
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
        .route("/payments/new", get(payments::flow::payment_new_page))
        .route(
            "/payments/methods",
            get(payments::saved_cards::payment_methods_page),
        )
        .route(
            "/payments/success",
            get(payments::flow::payment_success_page),
        )
        .route("/payments/cancel", get(payments::flow::payment_cancel_page))
        // Receipts. Restorable scope: Expired members can still pull
        // historical receipts (for tax filing) even though they aren't
        // currently Active.
        .route("/payments/receipts", get(payments::receipts::receipts_page))
        .route(
            "/payments/:payment_id/receipt",
            get(payments::receipts::receipt_page),
        )
        // Payment/card APIs
        .route(
            "/api/payments/checkout",
            post(payments::checkout::checkout_api),
        )
        .route(
            "/api/payments/charge-saved",
            post(payments::checkout::charge_saved_card_api),
        )
        .route(
            "/api/payments/list",
            get(payments::views::payments_list_api),
        )
        .route(
            "/api/payments/summary",
            get(payments::views::payments_summary_api),
        )
        .route(
            "/api/payments/dues-status",
            get(payments::views::dues_status_api),
        )
        .route("/api/payments/next-due", get(payments::views::next_due_api))
        .route(
            "/api/payments/cards",
            get(payments::saved_cards::saved_cards_html_api),
        )
        .route(
            "/api/payments/cards/:card_id",
            delete(payments::saved_cards::delete_card_api),
        )
        .route(
            "/api/payments/cards/:card_id/default",
            put(payments::saved_cards::set_default_card_api),
        )
        .route(
            "/api/payments/auto-renew",
            post(payments::saved_cards::update_auto_renew_api),
        )
        // CSRF is enforced at the application root; only the auth gate
        // is layered per-router.
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
        .route("/payments", get(payments::views::payments_page))
        .route("/donate", get(donations::donate_page))
        .route("/profile", get(profile::profile_page))
        .route("/profile", post(profile::update_profile))
        .route("/profile/password", post(profile::update_password))
        .route("/profile/security", get(security::security_page))
        .route(
            "/profile/security/totp/enroll/start",
            post(security::enroll_start),
        )
        .route(
            "/profile/security/totp/enroll/confirm",
            post(security::enroll_confirm),
        )
        .route("/profile/security/totp/disable", post(security::disable))
        .route(
            "/profile/security/totp/recovery-codes/regenerate",
            post(security::regenerate_recovery_codes),
        )
        // API endpoints (HTMX fragments) — for Active members only
        .route("/api/events/upcoming", get(dashboard::upcoming_events))
        .route("/api/events/list", get(events::events_list_api))
        .route("/api/events/:id/rsvp", post(events::rsvp_event))
        .route("/api/events/:id/cancel", post(events::cancel_rsvp_event))
        .route(
            "/api/announcements/list",
            get(announcements::announcements_list_api),
        )
        .route("/api/payments/recent", get(dashboard::recent_payments))
        .route("/api/donate", post(donations::donate_api))
        // CSRF is enforced at the application root; only the auth gate
        // is layered per-router.
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
    pub id: uuid::Uuid,
    pub username: String,
    pub full_name: String,
    pub email: String,
    pub status: crate::domain::MemberStatus,
    pub membership_type: String,
    pub joined_at: chrono::DateTime<chrono::Utc>,
    pub dues_paid_until: Option<chrono::DateTime<chrono::Utc>>,
}

pub fn is_admin(member: &crate::domain::Member) -> bool {
    member.is_admin
}
