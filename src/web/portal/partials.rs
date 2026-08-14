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

/// True for payments the member's own history should show — the ones
/// where money actually moved. `Pending`/`Failed` rows are abandoned
/// checkouts and transient failures (issue #120): noise in the member
/// view, so they're hidden here and remain visible only to admins.
pub fn is_member_visible(status: &crate::domain::PaymentStatus) -> bool {
    use crate::domain::PaymentStatus;
    matches!(status, PaymentStatus::Completed | PaymentStatus::Refunded)
}

/// The member-visible read of a member's payments, newest first. Every
/// member-facing surface that lists payments reads through here, so a
/// new surface gets the filter without remembering to apply it — the
/// first pass at issue #120 filtered the Payments page and left the
/// dashboard showing the rows that page hides.
///
/// Filtering here also means it happens before any caller-side limit: a
/// surface showing "the most recent five" can't truncate settled
/// payments out of view behind five abandoned checkouts.
///
/// Admin surfaces read `find_by_member` directly — they must keep
/// seeing `Pending`/`Failed`, which is how an abandoned checkout gets
/// diagnosed.
///
/// A read failure degrades to an empty list: callers are HTMX fragments
/// that swap into a live page, and an empty payment widget beats an
/// error card wedged into the dashboard. It's logged, because on the
/// rendered page an empty list is indistinguishable from "no payments".
pub async fn member_visible_payments(
    payment_repo: &dyn crate::repository::PaymentRepository,
    member_id: uuid::Uuid,
) -> Vec<crate::domain::Payment> {
    payment_repo
        .find_by_member(member_id)
        .await
        .unwrap_or_else(|e| {
            tracing::error!(
                "member payment history read failed for member {}: {}",
                member_id,
                e
            );
            Vec::new()
        })
        .into_iter()
        .filter(|p| is_member_visible(&p.status))
        .collect()
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
        // paid_at is when the money moved (correct for imported/backdated
        // rows); created_at is just row insertion. Same convention as
        // receipts + finance reports.
        date: payment
            .paid_at
            .unwrap_or(payment.created_at)
            .format("%B %d, %Y")
            .to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Payer, Payment, PaymentKind, PaymentMethod, PaymentStatus};

    fn payment_with(status: PaymentStatus) -> Payment {
        let now = chrono::Utc::now();
        Payment {
            id: uuid::Uuid::new_v4(),
            payer: Payer::Member(uuid::Uuid::new_v4()),
            amount_cents: 5000,
            currency: "USD".to_string(),
            status,
            payment_method: PaymentMethod::Stripe,
            kind: PaymentKind::Membership,
            external_id: None,
            description: String::new(),
            paid_at: Some(now),
            created_at: now,
            updated_at: now,
        }
    }

    // Every member payment list filters through `is_member_visible`
    // via `member_visible_payments`; the admin list does not. These two
    // tests pin both halves of issue #120 at the mapping level — the
    // per-surface tests live with their handlers.

    #[test]
    fn member_view_shows_only_settled_payments() {
        let payments = [
            payment_with(PaymentStatus::Completed),
            payment_with(PaymentStatus::Refunded),
            payment_with(PaymentStatus::Pending),
            payment_with(PaymentStatus::Failed),
        ];

        let shown: Vec<&str> = payments
            .iter()
            .filter(|p| is_member_visible(&p.status))
            .map(member_payment_row_from)
            .map(|row| row.status)
            .collect();

        assert_eq!(shown, ["Completed", "Refunded"]);
    }

    #[test]
    fn admin_view_unfiltered_shows_all_statuses() {
        // Admin path maps every payment without the visibility filter,
        // so Pending/Failed remain visible for support/reconciliation.
        let payments = [
            payment_with(PaymentStatus::Completed),
            payment_with(PaymentStatus::Refunded),
            payment_with(PaymentStatus::Pending),
            payment_with(PaymentStatus::Failed),
        ];

        let shown: Vec<&str> = payments
            .iter()
            .map(member_payment_row_from)
            .map(|row| row.status)
            .collect();

        assert_eq!(shown, ["Completed", "Refunded", "Pending", "Failed"]);
    }
}
