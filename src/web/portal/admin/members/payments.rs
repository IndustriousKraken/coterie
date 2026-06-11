use std::sync::Arc;

use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Extension,
};
use serde::Deserialize;

use crate::{
    api::middleware::auth::{CurrentUser, SessionInfo},
    auth::CsrfService,
    repository::{DonationCampaignRepository, MemberRepository},
    service::{membership_type_service::MembershipTypeService, payment_service::PaymentService},
    web::templates::{BaseContext, HtmlTemplate},
};

#[derive(askama::Template)]
#[template(path = "admin/record_payment.html")]
pub struct RecordPaymentTemplate {
    pub base: BaseContext,
    pub member_id: String,
    pub member_name: String,
    pub member_email: String,
    pub membership_types: Vec<RecordPaymentMembershipType>,
    pub donation_campaigns: Vec<RecordPaymentCampaign>,
    /// The slug of the member's current membership type, so the form
    /// can pre-select it. Empty if not assigned.
    pub current_membership_slug: String,
    pub flash_error: Option<String>,
}

pub struct RecordPaymentMembershipType {
    pub slug: String,
    pub name: String,
    pub fee_display: String,
    pub billing_period: String,
}

pub struct RecordPaymentCampaign {
    pub id: String,
    pub name: String,
}

pub async fn admin_record_payment_page(
    State(member_repo): State<Arc<dyn MemberRepository>>,
    State(membership_type_service): State<Arc<MembershipTypeService>>,
    State(donation_campaign_repo): State<Arc<dyn DonationCampaignRepository>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Path(member_id): Path<String>,
) -> axum::response::Response {
    render_record_payment(
        &member_repo,
        &membership_type_service,
        &donation_campaign_repo,
        &csrf_service,
        &current_user,
        &session_info,
        &member_id,
        None,
    )
    .await
}

#[derive(Debug, Deserialize)]
pub struct RecordPaymentForm {
    #[allow(dead_code)]
    pub csrf_token: String,
    /// "membership" | "donation" | "other"
    pub payment_type: String,
    pub amount: String,
    pub description: String,
    /// Set when payment_type=membership
    #[serde(default)]
    pub membership_type_slug: String,
    /// Set when payment_type=donation
    #[serde(default)]
    pub donation_campaign_id: String,
}

pub async fn admin_record_payment_submit(
    State(member_repo): State<Arc<dyn MemberRepository>>,
    State(membership_type_service): State<Arc<MembershipTypeService>>,
    State(donation_campaign_repo): State<Arc<dyn DonationCampaignRepository>>,
    State(payment_service): State<Arc<PaymentService>>,
    State(billing_service): State<Arc<crate::service::billing_service::BillingService>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Path(member_id): Path<String>,
    axum::Form(form): axum::Form<RecordPaymentForm>,
) -> axum::response::Response {
    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Redirect::to("/portal/admin/members").into_response(),
    };

    let err = |msg: String| {
        render_record_payment(
            &member_repo,
            &membership_type_service,
            &donation_campaign_repo,
            &csrf_service,
            &current_user,
            &session_info,
            &member_id,
            Some(msg),
        )
    };

    // Parse dollars → cents. Accept "100" or "100.00" or "100.5".
    let amount_cents = match parse_dollars_to_cents(&form.amount) {
        Some(c) if c > 0 || form.payment_type == "membership" => c,
        _ => return err("Amount must be a positive dollar amount.".to_string()).await,
    };
    if amount_cents > crate::domain::MAX_PAYMENT_CENTS {
        return err(format!(
            "Amount exceeds the ${} cap on a single payment — \
             split it into multiple records if intentional.",
            crate::domain::MAX_PAYMENT_CENTS / 100,
        ))
        .await;
    }

    use crate::domain::{PaymentKind, PaymentMethod};
    use crate::service::payment_service::RecordManualPaymentInput;

    // Wire-format parsing: the form's `payment_type` string + the
    // form's separate campaign-id string become a typed `PaymentKind`.
    // Empty/invalid campaign id is rejected here (form-shape validation);
    // existence is checked by `PaymentService::record_manual`.
    let kind = match form.payment_type.as_str() {
        "membership" => PaymentKind::Membership,
        "donation" => {
            let cid_str = form.donation_campaign_id.trim();
            if cid_str.is_empty() {
                return err("Donation requires a campaign selection.".to_string()).await;
            }
            let cid = match uuid::Uuid::parse_str(cid_str) {
                Ok(cid) => cid,
                Err(_) => return err("Invalid campaign id.".to_string()).await,
            };
            PaymentKind::Donation {
                campaign_id: Some(cid),
            }
        }
        "other" => PaymentKind::Other,
        _ => return err("Invalid payment type.".to_string()).await,
    };

    let description = if form.description.trim().is_empty() {
        match kind {
            PaymentKind::Membership => "Manual membership payment".to_string(),
            PaymentKind::Donation { .. } => "Donation".to_string(),
            PaymentKind::Other => "Manual payment".to_string(),
        }
    } else {
        form.description.clone()
    };

    let slug_for_dues =
        if matches!(kind, PaymentKind::Membership) && !form.membership_type_slug.is_empty() {
            Some(form.membership_type_slug.clone())
        } else {
            None
        };
    if let Err(e) = payment_service
        .record_manual(
            RecordManualPaymentInput {
                member_id: id,
                amount_cents,
                kind,
                description,
                payment_method: PaymentMethod::Manual,
                membership_type_slug: slug_for_dues,
                actor_id: current_user.member.id,
            },
            &billing_service,
        )
        .await
    {
        return err(format!("Failed to record payment: {}", e)).await;
    }

    // PaymentService emits the audit event itself, so the handler is
    // done once record_manual returns Ok.
    axum::response::Redirect::to(&format!("/portal/admin/members/{}", id)).into_response()
}

/// "100", "100.00", "100.5" → 10000, 10000, 10050. Returns None on
/// junk input or negative values. Refuses more than 2 decimal places
/// to prevent silent rounding.
fn parse_dollars_to_cents(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (whole, frac) = match s.split_once('.') {
        Some((w, f)) => (w, f),
        None => (s, ""),
    };
    if frac.len() > 2 || !frac.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let whole: i64 = whole.parse().ok()?;
    if whole < 0 {
        return None;
    }
    let frac_padded = format!("{:0<2}", frac);
    let frac: i64 = if frac_padded.is_empty() {
        0
    } else {
        frac_padded.parse().ok()?
    };
    whole.checked_mul(100)?.checked_add(frac)
}

/// Render the record-payment page for `member_id`, optionally with a
/// flash error message. Shared between the GET page and POST validation
/// failures (the latter re-renders the same form with the error shown).
#[allow(clippy::too_many_arguments)]
async fn render_record_payment(
    member_repo: &Arc<dyn MemberRepository>,
    membership_type_service: &Arc<MembershipTypeService>,
    donation_campaign_repo: &Arc<dyn DonationCampaignRepository>,
    csrf_service: &CsrfService,
    current_user: &CurrentUser,
    session_info: &SessionInfo,
    member_id: &str,
    flash_error: Option<String>,
) -> axum::response::Response {
    let id = match uuid::Uuid::parse_str(member_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Redirect::to("/portal/admin/members").into_response(),
    };
    let member = match member_repo.find_by_id(id).await {
        Ok(Some(m)) => m,
        _ => return axum::response::Redirect::to("/portal/admin/members").into_response(),
    };

    let base = BaseContext::for_member(csrf_service, current_user, session_info).await;

    let membership_types = membership_type_service
        .list(false)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|mt| RecordPaymentMembershipType {
            slug: mt.slug,
            name: mt.name,
            fee_display: format!("{:.2}", mt.fee_cents as f64 / 100.0),
            billing_period: mt.billing_period,
        })
        .collect();

    let donation_campaigns = donation_campaign_repo
        .list_active()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|c| RecordPaymentCampaign {
            id: c.id.to_string(),
            name: c.name,
        })
        .collect();

    let current_membership_slug = membership_type_service
        .get(member.membership_type_id)
        .await
        .ok()
        .flatten()
        .map(|mt| mt.slug)
        .unwrap_or_default();

    HtmlTemplate(RecordPaymentTemplate {
        base,
        member_id: member.id.to_string(),
        member_name: member.full_name.clone(),
        member_email: member.email.clone(),
        membership_types,
        donation_campaigns,
        current_membership_slug,
        flash_error,
    })
    .into_response()
}
