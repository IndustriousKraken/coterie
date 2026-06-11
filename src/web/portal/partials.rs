//! Reusable Askama partials for member-side portal HTMX fragments.
//!
//! Mirrors `admin/partials.rs` for the non-admin half of the portal.
//! Markup edits happen in `templates/portal/_*.html`; this file is
//! data-assembly + render plumbing only.

use askama::Template;
use axum::response::Html;

// --------------------------------------------------------------------
// Member's own payment history
// --------------------------------------------------------------------

pub struct MemberPaymentRow {
    pub description: String,
    pub date: String,
    pub amount: String,
    pub status: &'static str,
}

#[derive(Template)]
#[template(path = "portal/_member_payment_list.html")]
pub struct MemberPaymentListTemplate {
    pub rows: Vec<MemberPaymentRow>,
}

pub fn member_payment_list(rows: Vec<MemberPaymentRow>) -> Html<String> {
    let tmpl = MemberPaymentListTemplate { rows };
    Html(tmpl.render().unwrap_or_else(|e| {
        tracing::error!("member_payment_list template render failed: {}", e);
        format!("<div class=\"p-6 text-center text-red-600\">Render error</div>")
    }))
}

pub fn member_payment_row_from(payment: &crate::domain::Payment) -> MemberPaymentRow {
    use crate::domain::PaymentStatus;
    let status = match payment.status {
        PaymentStatus::Completed => "Completed",
        PaymentStatus::Pending => "Pending",
        PaymentStatus::Failed => "Failed",
        PaymentStatus::Refunded => "Refunded",
    };

    let description = if payment.description.is_empty() {
        "Membership dues".to_string()
    } else {
        payment.description.clone()
    };

    MemberPaymentRow {
        description,
        date: payment.created_at.format("%B %d, %Y").to_string(),
        amount: format!("{:.2}", payment.amount_cents as f64 / 100.0),
        status,
    }
}

// --------------------------------------------------------------------
// Saved payment-method list
// --------------------------------------------------------------------

pub struct SavedCardRow {
    pub id: String,
    pub display_name: String,
    pub exp_display: String,
    pub is_default: bool,
    pub is_expired: bool,
}

#[derive(Template)]
#[template(path = "portal/_saved_card_list.html")]
pub struct SavedCardListTemplate {
    pub rows: Vec<SavedCardRow>,
}

pub fn saved_card_list(rows: Vec<SavedCardRow>) -> Html<String> {
    let tmpl = SavedCardListTemplate { rows };
    Html(tmpl.render().unwrap_or_else(|e| {
        tracing::error!("saved_card_list template render failed: {}", e);
        format!("<div class=\"p-6 text-center text-red-600\">Render error</div>")
    }))
}

pub fn saved_card_row_from(card: &crate::domain::SavedCard) -> SavedCardRow {
    SavedCardRow {
        id: card.id.to_string(),
        display_name: card.display_name(),
        exp_display: card.exp_display(),
        is_default: card.is_default,
        is_expired: card.is_expired(),
    }
}

// --------------------------------------------------------------------
// Tiny dues-status pill
// --------------------------------------------------------------------

#[derive(Template)]
#[template(path = "portal/_dues_status_pill.html")]
pub struct DuesStatusPillTemplate {
    /// `"current" | "expired" | "unpaid"`.
    pub status: &'static str,
}

pub fn dues_status_pill(status: &'static str) -> Html<String> {
    let tmpl = DuesStatusPillTemplate { status };
    Html(tmpl.render().unwrap_or_else(|e| {
        tracing::error!("dues_status_pill template render failed: {}", e);
        format!("<span class=\"text-yellow-600\">Unpaid</span>")
    }))
}
