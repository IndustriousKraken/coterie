//! Admin UI for billing operations. Currently scoped to the
//! Stripe-subscription → Coterie-managed migration: a one-shot
//! bulk-migrate button + a per-member count so the admin can see
//! the work to be done.

use askama::Template;
use axum::{
    extract::State,
    response::{IntoResponse, Redirect, Response},
    Extension,
};

use crate::{
    api::{
        middleware::auth::{CurrentUser, SessionInfo},
        state::AppState,
    },
    web::templates::{HtmlTemplate, UserInfo},
};
use crate::web::portal::is_admin;

#[derive(Template)]
#[template(path = "admin/billing_settings.html")]
pub struct AdminBillingTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub csrf_token: String,
    pub stripe_subscription_count: i64,
    pub stripe_enabled: bool,
    pub flash_success: Option<String>,
    pub flash_error: Option<String>,
    /// Last migration run summary, if any (rendered after a POST).
    pub last_succeeded: Option<u32>,
    pub last_skipped: Option<u32>,
    pub last_failed: Vec<(String, String)>,
}

pub async fn billing_settings_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> Response {
    render_page(state, current_user, session_info, RenderArgs::default()).await
}

#[derive(Default)]
struct RenderArgs {
    flash_success: Option<String>,
    flash_error: Option<String>,
    last_succeeded: Option<u32>,
    last_skipped: Option<u32>,
    last_failed: Vec<(String, String)>,
}

async fn render_page(
    state: AppState,
    current_user: CurrentUser,
    session_info: SessionInfo,
    args: RenderArgs,
) -> Response {
    if !is_admin(&current_user.member) {
        return Redirect::to("/portal/dashboard").into_response();
    }

    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let csrf_token = state.service_context.csrf_service
        .generate_token(&session_info.session_id)
        .await
        .unwrap_or_default();

    let stripe_subscription_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM members WHERE billing_mode = 'stripe_subscription'",
    )
    .fetch_one(&state.service_context.db_pool)
    .await
    .unwrap_or(0);

    HtmlTemplate(AdminBillingTemplate {
        current_user: Some(user_info),
        is_admin: true,
        csrf_token,
        stripe_subscription_count,
        stripe_enabled: state.stripe_client.is_some(),
        flash_success: args.flash_success,
        flash_error: args.flash_error,
        last_succeeded: args.last_succeeded,
        last_skipped: args.last_skipped,
        last_failed: args.last_failed,
    }).into_response()
}

/// Run the bulk migration of every member on `stripe_subscription`
/// to `coterie_managed`. Synchronous — fine for the typical Coterie
/// deployment where stripe-sub members number in the dozens at most.
/// If we ever have hundreds, move to a background task and an
/// HTMX-polled progress page.
pub async fn bulk_migrate_stripe_subs(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> Response {
    if !is_admin(&current_user.member) {
        return Redirect::to("/portal/dashboard").into_response();
    }

    if state.stripe_client.is_none() {
        return render_page(
            state, current_user, session_info,
            RenderArgs {
                flash_error: Some(
                    "Stripe isn't configured. Add credentials before running migration.".into()
                ),
                ..Default::default()
            },
        ).await;
    }

    let billing_service = state.service_context.billing_service(
        state.stripe_client.clone(),
        state.settings.server.base_url.clone(),
    );

    let summary = billing_service.bulk_migrate_stripe_subscriptions().await;

    state.service_context.audit_service.log(
        Some(current_user.member.id),
        "bulk_migrate_stripe_subscriptions",
        "billing",
        "all",
        None,
        Some(&format!(
            "succeeded={}, skipped={}, failed={}",
            summary.succeeded, summary.skipped, summary.failed.len(),
        )),
        None,
    ).await;

    let flash_success = if summary.failed.is_empty() {
        Some(format!(
            "Migrated {} member(s) to Coterie-managed auto-renew. {} skipped.",
            summary.succeeded, summary.skipped,
        ))
    } else {
        None
    };
    let flash_error = if !summary.failed.is_empty() {
        Some(format!(
            "{} member(s) migrated, but {} failed — see details below.",
            summary.succeeded, summary.failed.len(),
        ))
    } else {
        None
    };

    let last_failed: Vec<(String, String)> = summary.failed
        .into_iter()
        .map(|(id, err)| (id.to_string(), err))
        .collect();

    render_page(
        state, current_user, session_info,
        RenderArgs {
            flash_success,
            flash_error,
            last_succeeded: Some(summary.succeeded),
            last_skipped: Some(summary.skipped),
            last_failed,
        },
    ).await
}
