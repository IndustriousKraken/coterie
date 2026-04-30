//! Reusable Askama partials for admin HTMX fragments.
//!
//! These exist so the inline-`format!`-string HTML that used to live
//! in handlers can move to template files. Editing markup or styles
//! happens in `templates/admin/_*.html`, not in `.rs` files; the
//! handler code becomes data assembly only.

use askama::Template;
use axum::response::Html;

/// Result panel rendered after an admin HTMX action — the small
/// green/red/yellow div that appears under a button. `kind` is one of
/// `"success" | "error" | "warning"`. When `autoreload` is true, the
/// panel includes a 1.5s setTimeout that reloads the page so the
/// underlying row reflects the new server state.
#[derive(Template)]
#[template(path = "admin/_admin_alert.html")]
pub struct AdminAlertTemplate<'a> {
    pub kind: &'static str,
    pub message: &'a str,
    pub autoreload: bool,
}

/// Render an admin-alert fragment. Used as the HTMX response body
/// for handlers whose only job is to flash success/failure.
pub fn admin_alert(kind: &'static str, message: &str, autoreload: bool) -> Html<String> {
    let tmpl = AdminAlertTemplate { kind, message, autoreload };
    Html(tmpl.render().unwrap_or_else(|e| {
        // Render failure on a fragment is operator-fatal but caller-
        // benign — log it and ship a plain-text fallback so the user
        // sees *something* go through.
        tracing::error!("admin_alert template render failed: {}", e);
        format!("<div class=\"p-3 bg-red-50 text-red-800 rounded-md text-sm\">{}</div>", message)
    }))
}

// --------------------------------------------------------------------
// Member-row HTMX swap
// --------------------------------------------------------------------

/// One row in the admin members table, rendered as the HTMX response
/// body for activate / suspend / dues actions. `flash` selects which
/// of three styled variants to render:
///
///   - `"active"`    → green-tinted row, "Activated!" action badge
///   - `"suspended"` → yellow-tinted row, "Suspended" label
///   - `"dues"`      → neutral row, "Updated" badge (used when only
///                      dues_paid_until changed and status held)
///
/// Keep markup in sync with `templates/admin/members_table.html` —
/// the row layout (column order, badge style) must match the
/// initial-render template or the swap will jump visually.
#[derive(Template)]
#[template(path = "admin/_member_row_flash.html")]
pub struct MemberRowFlashTemplate {
    pub flash: &'static str,
    pub initials: String,
    pub full_name: String,
    pub email: String,
    pub username: String,
    pub status: String,
    pub membership_type: String,
    pub joined_at: String,
    pub dues_paid_until: String,
}

/// Build a `MemberRowFlashTemplate` from a `Member`.
pub fn member_row_flash(
    member: &crate::domain::Member,
    flash: &'static str,
) -> Html<String> {
    let initials: String = member.full_name
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase();

    let tmpl = MemberRowFlashTemplate {
        flash,
        initials,
        full_name: member.full_name.clone(),
        email: member.email.clone(),
        username: member.username.clone(),
        status: member.status.as_str().to_string(),
        membership_type: member.membership_type.as_str().to_string(),
        joined_at: member.joined_at.format("%b %d, %Y").to_string(),
        dues_paid_until: member.dues_paid_until
            .map(|d| d.format("%b %d, %Y").to_string())
            .unwrap_or_else(|| "—".to_string()),
    };

    Html(tmpl.render().unwrap_or_else(|e| {
        tracing::error!("member_row_flash template render failed: {}", e);
        format!(
            "<tr><td colspan='6' class='px-6 py-4 text-red-600'>Render error</td></tr>"
        )
    }))
}

/// Error placeholder row for the members table. Returned when an
/// admin handler can't load the member to render a real row (bad
/// UUID, repo error). Auto-escapes `message`.
pub fn member_row_error(message: &str) -> Html<String> {
    Html(format!(
        "<tr><td colspan='6' class='px-6 py-4 text-red-600'>{}</td></tr>",
        crate::web::escape_html(message),
    ))
}

// --------------------------------------------------------------------
// Admin member-detail payment-history list
// --------------------------------------------------------------------

/// One row's worth of pre-rendered display values. The handler
/// flattens domain `Payment`s into these (status as a string,
/// amount pre-formatted, refund-button gating decided up front)
/// so the template only does string interpolation, not branching
/// on domain enums.
pub struct AdminPaymentRow {
    pub id: String,
    pub description: String,
    pub date: String,
    pub amount: String,
    pub status: &'static str,
    pub show_refund: bool,
    pub refund_confirm: String,
}

#[derive(Template)]
#[template(path = "admin/_admin_payment_list.html")]
pub struct AdminPaymentListTemplate {
    pub rows: Vec<AdminPaymentRow>,
}

/// Render the admin member-detail payments list. Returns the empty-
/// state message when the member has no payments on file.
pub fn admin_payment_list(rows: Vec<AdminPaymentRow>) -> Html<String> {
    let tmpl = AdminPaymentListTemplate { rows };
    Html(tmpl.render().unwrap_or_else(|e| {
        tracing::error!("admin_payment_list template render failed: {}", e);
        format!("<div class=\"p-6 text-center text-red-600\">Render error</div>")
    }))
}

/// Build an `AdminPaymentRow` view-model from a domain `Payment`.
/// Refund-button gating: only Completed Stripe / Manual rows. Waived
/// rows are $0 — nothing to give back. Already-refunded rows
/// obviously get no button.
pub fn admin_payment_row_from(payment: &crate::domain::Payment) -> AdminPaymentRow {
    use crate::domain::{PaymentMethod, PaymentStatus};
    let status = match payment.status {
        PaymentStatus::Completed => "Completed",
        PaymentStatus::Pending => "Pending",
        PaymentStatus::Failed => "Failed",
        PaymentStatus::Refunded => "Refunded",
    };

    let show_refund = payment.status == PaymentStatus::Completed
        && payment.payment_method != PaymentMethod::Waived;

    let amount_dollars = payment.amount_cents as f64 / 100.0;
    let refund_confirm = if show_refund {
        match payment.payment_method {
            PaymentMethod::Stripe => format!(
                "Issue a full Stripe refund of ${:.2}? This is irreversible.",
                amount_dollars,
            ),
            _ => format!(
                "Mark this ${:.2} payment as Refunded? (No external system will be touched — refund the cash/check yourself.)",
                amount_dollars,
            ),
        }
    } else {
        String::new()
    };

    let description = if payment.description.is_empty() {
        "Membership dues".to_string()
    } else {
        payment.description.clone()
    };

    AdminPaymentRow {
        id: payment.id.to_string(),
        description,
        date: payment.created_at.format("%B %d, %Y").to_string(),
        amount: format!("{:.2}", amount_dollars),
        status,
        show_refund,
        refund_confirm,
    }
}
