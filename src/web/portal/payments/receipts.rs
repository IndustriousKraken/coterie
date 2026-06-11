use std::sync::Arc;

use askama::Template;
use axum::{extract::State, response::IntoResponse, Extension};

use crate::{
    api::middleware::auth::{CurrentUser, SessionInfo},
    auth::CsrfService,
    error::AppError,
    repository::{DonationCampaignRepository, PaymentRepository},
    service::settings_service::SettingsService,
    web::templates::{BaseContext, HtmlTemplate},
};

// =============================================================================
// Member-facing receipts
// =============================================================================
//
// Two surfaces:
//   /portal/payments/receipts          — yearly aggregation page
//   /portal/payments/:id/receipt       — printable single-payment receipt
//
// Both are member-self-service: a member can only see their own
// payments. Admins see member receipts via the admin payment views.
//
// The yearly page splits dues from donations because the totals serve
// different purposes — donation totals go to a 501c3-aware accountant
// for tax filing; dues totals are personal records. Refunded payments
// are filtered out: the money never landed, so the receipt would
// mislead.

#[derive(Template)]
#[template(path = "portal/receipts.html")]
pub struct ReceiptsTemplate {
    pub base: BaseContext,
    pub member_full_name: String,
    pub years: Vec<ReceiptYearDisplay>,
}

pub struct ReceiptYearDisplay {
    pub year: i32,
    pub dues_total_display: String,
    pub donations_total_display: String,
    pub items: Vec<ReceiptLineDisplay>,
}

pub struct ReceiptLineDisplay {
    pub payment_id: String,
    pub date: String,
    pub description: String,
    pub kind_label: String, // "Dues" | "Donation" | "Other"
    pub amount_display: String,
}

#[derive(Template)]
#[template(path = "portal/receipt.html")]
pub struct ReceiptTemplate {
    // Org letterhead. Empty fields render as blank lines and the
    // template hides them via {% if %} guards.
    pub org_name: String,
    pub org_address: String,
    pub org_contact_email: String,
    pub org_website_url: String,
    pub org_tax_id: String,

    // Receipt itself
    pub payment_id: String,
    pub recipient_name: String,
    pub recipient_email: String,
    pub date: String,
    pub amount_display: String,
    pub kind_label: String, // "Dues" | "Donation" | "Other"
    pub description: String,
    pub campaign: Option<String>,
    pub payment_method_label: String, // "Card via Stripe" | "Manual" | "Waived"
    pub generated_on: String,
}

/// Tax-year aggregation page. Lists every Completed payment grouped
/// by calendar year of `paid_at` (falling back to created_at if
/// paid_at is missing — old manual records sometimes lack it). Per
/// year, the page totals dues separately from donations so the donor-
/// or-member can hand the right number to the right place.
pub async fn receipts_page(
    State(csrf_service): State<Arc<CsrfService>>,
    State(payment_repo): State<Arc<dyn PaymentRepository>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session): Extension<SessionInfo>,
) -> Result<axum::response::Response, AppError> {
    use crate::domain::{PaymentKind, PaymentStatus};
    use std::collections::BTreeMap;

    let payments = payment_repo.find_by_member(current_user.member.id).await?;

    // Group by year. BTreeMap so years come out sorted; we'll reverse
    // into newest-first for display below.
    let mut by_year: BTreeMap<i32, Vec<crate::domain::Payment>> = BTreeMap::new();
    for p in payments {
        // Only completed payments have a real receipt. Pending hasn't
        // landed; Failed didn't land; Refunded was reversed and would
        // mislead an accountant who tallies the receipt total.
        if p.status != PaymentStatus::Completed {
            continue;
        }
        let when = p.paid_at.unwrap_or(p.created_at);
        let year = when.format("%Y").to_string().parse::<i32>().unwrap_or(0);
        by_year.entry(year).or_default().push(p);
    }

    let mut years: Vec<ReceiptYearDisplay> = by_year
        .into_iter()
        .map(|(year, items)| {
            let mut dues_cents: i64 = 0;
            let mut donations_cents: i64 = 0;
            let mut lines: Vec<ReceiptLineDisplay> = items
                .into_iter()
                .map(|p| {
                    let kind_label = match p.kind {
                        PaymentKind::Membership => {
                            dues_cents += p.amount_cents;
                            "Dues"
                        }
                        PaymentKind::Donation { .. } => {
                            donations_cents += p.amount_cents;
                            "Donation"
                        }
                        PaymentKind::Other => "Other",
                    }
                    .to_string();

                    let when = p.paid_at.unwrap_or(p.created_at);
                    ReceiptLineDisplay {
                        payment_id: p.id.to_string(),
                        date: when.format("%Y-%m-%d").to_string(),
                        description: p.description.clone(),
                        kind_label,
                        amount_display: format!("${:.2}", p.amount_cents as f64 / 100.0),
                    }
                })
                .collect();

            // Newest-first within the year.
            lines.sort_by(|a, b| b.date.cmp(&a.date));

            ReceiptYearDisplay {
                year,
                dues_total_display: format!("${:.2}", dues_cents as f64 / 100.0),
                donations_total_display: format!("${:.2}", donations_cents as f64 / 100.0),
                items: lines,
            }
        })
        .collect();

    // Newest year first.
    years.sort_by(|a, b| b.year.cmp(&a.year));

    let template = ReceiptsTemplate {
        base: BaseContext::for_member(&csrf_service, &current_user, &session).await,
        member_full_name: current_user.member.full_name.clone(),
        years,
    };
    Ok(HtmlTemplate(template).into_response())
}

/// Printable single-payment receipt. Standalone HTML (no portal nav),
/// styled for both screen and print. Member can only see their own
/// receipts; refunded / pending / failed payments return 404 (no
/// receipt to show).
pub async fn receipt_page(
    State(payment_repo): State<Arc<dyn PaymentRepository>>,
    State(settings_service): State<Arc<SettingsService>>,
    State(donation_campaign_repo): State<Arc<dyn DonationCampaignRepository>>,
    Extension(current_user): Extension<CurrentUser>,
    axum::extract::Path(payment_id): axum::extract::Path<uuid::Uuid>,
) -> Result<axum::response::Response, AppError> {
    use crate::domain::{PaymentKind, PaymentMethod, PaymentStatus};

    let payment = payment_repo
        .find_by_id(payment_id)
        .await?
        .ok_or(AppError::NotFound("Receipt not found".to_string()))?;

    // Ownership check: must be the member's own payment. Public
    // donations (Payer::PublicDonor) aren't accessible through the
    // portal — those donors have no login.
    if payment.member_id() != Some(current_user.member.id) {
        // Don't leak existence of other members' payments — same code
        // as the absent case.
        return Err(AppError::NotFound("Receipt not found".to_string()));
    }

    if payment.status != PaymentStatus::Completed {
        return Err(AppError::NotFound("Receipt not found".to_string()));
    }

    let raw_org_name = settings_service
        .get_value("org.name")
        .await
        .unwrap_or_default();
    let org_name = if raw_org_name.is_empty() {
        "Coterie".to_string()
    } else {
        raw_org_name
    };
    let org_address = settings_service
        .get_value("org.address")
        .await
        .unwrap_or_default();
    let org_contact_email = settings_service
        .get_value("org.contact_email")
        .await
        .unwrap_or_default();
    let org_website_url = settings_service
        .get_value("org.website_url")
        .await
        .unwrap_or_default();
    let org_tax_id = settings_service
        .get_value("org.tax_id")
        .await
        .unwrap_or_default();

    let kind_label = match payment.kind {
        PaymentKind::Membership => "Dues",
        PaymentKind::Donation { .. } => "Donation",
        PaymentKind::Other => "Other",
    }
    .to_string();

    let payment_method_label = match payment.payment_method {
        PaymentMethod::Stripe => "Card via Stripe",
        PaymentMethod::Manual => "Manual",
        PaymentMethod::Waived => "Waived",
    }
    .to_string();

    let when = payment.paid_at.unwrap_or(payment.created_at);

    // Try to resolve campaign name when applicable.
    let campaign = if let Some(cid) = payment.kind.campaign_id() {
        donation_campaign_repo
            .find_by_id(cid)
            .await
            .ok()
            .flatten()
            .map(|c| c.name)
    } else {
        None
    };

    let template = ReceiptTemplate {
        org_name,
        org_address,
        org_contact_email,
        org_website_url,
        org_tax_id,
        payment_id: payment.id.to_string(),
        recipient_name: current_user.member.full_name.clone(),
        recipient_email: current_user.member.email.clone(),
        date: when.format("%B %-d, %Y").to_string(),
        amount_display: format!("${:.2}", payment.amount_cents as f64 / 100.0),
        kind_label,
        description: payment.description.clone(),
        campaign,
        payment_method_label,
        generated_on: chrono::Utc::now().format("%B %-d, %Y").to_string(),
    };
    Ok(HtmlTemplate(template).into_response())
}
