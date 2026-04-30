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
    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let csrf_token = state.service_context.csrf_service
        .generate_token(&session_info.session_id)
        .await
        .unwrap_or_default();

    let stripe_subscription_count = state.service_context.member_repo
        .count_by_billing_mode(crate::domain::BillingMode::StripeSubscription)
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

// =====================================================================
// Billing dashboard — read-only operator overview
//
// Three sections: upcoming scheduled (next 30 days), recent failures
// (last 90 days), revenue by month split into dues vs donations
// (last 12 months). Every row links to a per-member page where the
// actual remediation actions live; this page is observation, not
// action.
// =====================================================================

#[derive(Template)]
#[template(path = "admin/billing_dashboard.html")]
pub struct AdminBillingDashboardTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub csrf_token: String,
    pub upcoming: Vec<UpcomingScheduledRow>,
    pub failures: Vec<FailedScheduledRow>,
    pub months: Vec<MonthlyRevenueRow>,
    /// 30 / 90 / 12 — surfaced so the section copy stays in sync if
    /// we ever change the windows. (And so the template doesn't
    /// hardcode magic numbers separately.)
    pub upcoming_window_days: i64,
    pub failure_window_days: i64,
    pub revenue_window_months: u32,
}

pub struct UpcomingScheduledRow {
    pub member_id: String,
    pub member_name: String,
    pub due_date: String,
    pub amount_display: String,
    pub retry_count: i32,
    pub status: &'static str,
}

pub struct FailedScheduledRow {
    pub member_id: String,
    pub member_name: String,
    pub last_attempt_display: String,
    pub amount_display: String,
    pub retry_count: i32,
    pub failure_reason: String,
}

pub struct MonthlyRevenueRow {
    /// e.g. "2026-04"
    pub month_key: String,
    /// e.g. "April 2026"
    pub month_label: String,
    pub dues_dollars: String,
    pub dues_count: i64,
    pub donations_dollars: String,
    pub donations_count: i64,
    pub total_dollars: String,
}

const UPCOMING_WINDOW_DAYS: i64 = 30;
const FAILURE_WINDOW_DAYS: i64 = 90;
const REVENUE_WINDOW_MONTHS: u32 = 12;

pub async fn billing_dashboard_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> Response {
    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };
    let csrf_token = state.service_context.csrf_service
        .generate_token(&session_info.session_id).await
        .unwrap_or_else(|_| String::new());

    // ---- Section 1: upcoming scheduled (next 30 days) ----
    let now = chrono::Utc::now();
    let upcoming_cutoff = (now + chrono::Duration::days(UPCOMING_WINDOW_DAYS)).date_naive();
    let upcoming_raw = state.service_context.scheduled_payment_repo
        .find_pending_due_before(upcoming_cutoff).await
        .unwrap_or_default();
    let mut upcoming = Vec::with_capacity(upcoming_raw.len());
    for sp in upcoming_raw {
        let name = lookup_member_name(&state, sp.member_id).await;
        upcoming.push(UpcomingScheduledRow {
            member_id: sp.member_id.to_string(),
            member_name: name,
            due_date: sp.due_date.format("%b %d, %Y").to_string(),
            amount_display: format!("${:.2}", sp.amount_cents as f64 / 100.0),
            retry_count: sp.retry_count,
            status: match sp.status {
                crate::domain::ScheduledPaymentStatus::Pending => "Pending",
                crate::domain::ScheduledPaymentStatus::Processing => "Processing",
                crate::domain::ScheduledPaymentStatus::Completed => "Completed",
                crate::domain::ScheduledPaymentStatus::Failed => "Failed",
                crate::domain::ScheduledPaymentStatus::Canceled => "Canceled",
            },
        });
    }

    // ---- Section 2: recent failures (last 90 days) ----
    let failure_since = now - chrono::Duration::days(FAILURE_WINDOW_DAYS);
    let failures_raw = state.service_context.scheduled_payment_repo
        .list_failures_since(failure_since).await
        .unwrap_or_default();
    let mut failures = Vec::with_capacity(failures_raw.len());
    for sp in failures_raw {
        let name = lookup_member_name(&state, sp.member_id).await;
        failures.push(FailedScheduledRow {
            member_id: sp.member_id.to_string(),
            member_name: name,
            last_attempt_display: sp.last_attempt_at
                .map(|d| d.format("%b %d, %Y %H:%M UTC").to_string())
                .unwrap_or_else(|| "—".to_string()),
            amount_display: format!("${:.2}", sp.amount_cents as f64 / 100.0),
            retry_count: sp.retry_count,
            failure_reason: sp.failure_reason.unwrap_or_else(|| "—".to_string()),
        });
    }

    // ---- Section 3: revenue by month (12 months) ----
    let buckets = state.service_context.payment_repo
        .revenue_by_month(REVENUE_WINDOW_MONTHS).await
        .unwrap_or_default();
    let months = fold_revenue_buckets(buckets);

    HtmlTemplate(AdminBillingDashboardTemplate {
        current_user: Some(user_info),
        is_admin: true,
        csrf_token,
        upcoming,
        failures,
        months,
        upcoming_window_days: UPCOMING_WINDOW_DAYS,
        failure_window_days: FAILURE_WINDOW_DAYS,
        revenue_window_months: REVENUE_WINDOW_MONTHS,
    }).into_response()
}

/// Look up a member's display name. Falls back to the UUID prefix on
/// missing rows — a deleted member may still have outstanding
/// scheduled-payment rows referencing them, and the dashboard should
/// degrade gracefully rather than 500.
async fn lookup_member_name(state: &AppState, member_id: uuid::Uuid) -> String {
    state.service_context.member_repo
        .find_by_id(member_id).await.ok().flatten()
        .map(|m| m.full_name)
        .unwrap_or_else(|| format!("(deleted member {})", &member_id.to_string()[..8]))
}

/// Fold the flat (year, month, type) buckets into one row per month
/// with separate dues / donations totals. The flat list comes back
/// already sorted newest-first, so the order survives.
fn fold_revenue_buckets(buckets: Vec<crate::repository::MonthlyRevenue>) -> Vec<MonthlyRevenueRow> {
    // Stable insertion-ordered map: BTreeMap keyed on (year, month)
    // sorted DESC; we'd rather not pull in indexmap for one place.
    let mut accum: std::collections::BTreeMap<(i32, u32), [i64; 4]> =
        std::collections::BTreeMap::new();
    // [dues_cents, dues_count, donations_cents, donations_count]
    //
    // `b.payment_type` is the raw DB column string (the values match
    // `PaymentKind::as_str()`); unknown values are folded into the
    // dues bucket alongside "other".
    for b in buckets {
        let entry = accum.entry((b.year, b.month)).or_insert([0; 4]);
        match b.payment_type.as_str() {
            "donation" => {
                entry[2] += b.total_cents;
                entry[3] += b.payment_count;
            }
            // "membership" | "other" | anything else → operating revenue.
            // If "other" becomes meaningful (merch, event fees with
            // their own line) split it out here.
            _ => {
                entry[0] += b.total_cents;
                entry[1] += b.payment_count;
            }
        }
    }

    // Render newest-first. BTreeMap iterates ascending, so reverse.
    accum.into_iter().rev().map(|((year, month), [dc, dn, oc, on])| {
        let dollars = |c: i64| format!("${:.2}", c as f64 / 100.0);
        let total = dc + oc;
        MonthlyRevenueRow {
            month_key: format!("{:04}-{:02}", year, month),
            month_label: format!("{} {}", month_name(month), year),
            dues_dollars: dollars(dc),
            dues_count: dn,
            donations_dollars: dollars(oc),
            donations_count: on,
            total_dollars: dollars(total),
        }
    }).collect()
}

fn month_name(m: u32) -> &'static str {
    match m {
        1 => "January", 2 => "February", 3 => "March", 4 => "April",
        5 => "May", 6 => "June", 7 => "July", 8 => "August",
        9 => "September", 10 => "October", 11 => "November", 12 => "December",
        _ => "—",
    }
}
