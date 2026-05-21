//! Admin handlers operating on payments (not on members). Lives
//! separately from `admin/members.rs` so the file location matches
//! the URL path (`/portal/admin/payments/...`).
//!
//! Today: a single thin refund handler that parses the path UUID and
//! calls `PaymentAdminService::refund`. The orchestration chain
//! (rate-limit, claim, Stripe, audit, integration alert) lives in
//! the service.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse},
    Extension,
};

use crate::{
    api::middleware::auth::CurrentUser,
    config::Settings,
    service::payment_admin_service::PaymentAdminService,
};

/// Refund a previously-recorded payment. Parse-call-render only —
/// validation, atomic claim, Stripe call, audit, and integration
/// dispatch all live in `PaymentAdminService::refund`.
pub async fn admin_refund_payment(
    State(svc): State<Arc<PaymentAdminService>>,
    State(settings): State<Arc<Settings>>,
    Extension(current_user): Extension<CurrentUser>,
    headers: axum::http::HeaderMap,
    Path(payment_id): Path<String>,
) -> impl IntoResponse {
    let ip = crate::api::state::client_ip(&headers, settings.server.trust_forwarded_for());
    let payment_uuid = match uuid::Uuid::parse_str(&payment_id) {
        Ok(id) => id,
        Err(_) => return refund_result_html(false, "Invalid payment ID"),
    };
    match svc.refund(current_user.member.id, payment_uuid, ip).await {
        Ok(outcome) => refund_result_html(true, &outcome.detail),
        Err(e) => refund_result_html(false, e.user_message()),
    }
}

fn refund_result_html(ok: bool, detail: &str) -> Html<String> {
    let escaped = crate::web::escape_html(detail);
    let (bg, fg) = if ok {
        ("bg-green-50", "text-green-900")
    } else {
        ("bg-red-50", "text-red-900")
    };
    // On success we trigger a soft reload so the payments list re-renders
    // with the new Refunded badge. On failure we just show the message.
    let suffix = if ok {
        r#"<script>setTimeout(() => htmx.trigger('#payments-list', 'refresh'), 800);</script>"#
    } else {
        ""
    };
    Html(format!(
        r#"<div class="mt-2 p-3 {bg} {fg} rounded-md text-sm">{escaped}</div>{suffix}"#,
        bg = bg, fg = fg, escaped = escaped, suffix = suffix,
    ))
}
