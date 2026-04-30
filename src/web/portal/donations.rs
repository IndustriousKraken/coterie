use askama::Template;
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Extension,
    Json,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    api::{
        middleware::auth::{CurrentUser, SessionInfo},
        state::AppState,
    },
    domain::{Payer, Payment, PaymentKind, PaymentMethod, PaymentStatus},
    error::AppError,
    web::templates::{HtmlTemplate, UserInfo},
};
use super::is_admin;
use super::payments::SavedCardDisplay;

#[derive(Template)]
#[template(path = "portal/donate.html")]
pub struct DonateTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub stripe_enabled: bool,
    pub stripe_publishable_key: String,
    pub campaigns: Vec<CampaignDisplay>,
    pub saved_cards: Vec<SavedCardDisplay>,
    pub csrf_token: String,
}

pub struct CampaignDisplay {
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub goal_display: Option<String>,
    pub raised_display: String,
    pub progress_pct: u32,
}

pub async fn donate_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> impl IntoResponse {
    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let csrf_token = state.service_context.csrf_service
        .generate_token(&session_info.session_id)
        .await
        .unwrap_or_default();

    let stripe_enabled = state.stripe_client.is_some();
    let stripe_publishable_key = state.settings.stripe.publishable_key
        .clone()
        .unwrap_or_default();

    let saved_cards = state.service_context.saved_card_repo
        .find_by_member(current_user.member.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|c| SavedCardDisplay {
            id: c.id.to_string(),
            display_name: c.display_name(),
            exp_display: c.exp_display(),
            is_default: c.is_default,
        })
        .collect();

    // Load active campaigns with progress
    let campaigns_raw = state.service_context.donation_campaign_repo
        .list_active()
        .await
        .unwrap_or_default();

    let mut campaigns = Vec::new();
    for c in campaigns_raw {
        let raised = state.service_context.donation_campaign_repo
            .get_total_donated(c.id)
            .await
            .unwrap_or(0);

        let (goal_display, progress_pct) = if let Some(goal) = c.goal_cents {
            let pct = if goal > 0 { ((raised as f64 / goal as f64) * 100.0).min(100.0) as u32 } else { 0 };
            (Some(format!("{:.2}", goal as f64 / 100.0)), pct)
        } else {
            (None, 0)
        };

        campaigns.push(CampaignDisplay {
            name: c.name,
            slug: c.slug,
            description: c.description,
            goal_display,
            raised_display: format!("{:.2}", raised as f64 / 100.0),
            progress_pct,
        });
    }

    let template = DonateTemplate {
        current_user: Some(user_info),
        is_admin: is_admin(&current_user.member),
        stripe_enabled,
        stripe_publishable_key,
        campaigns,
        saved_cards,
        csrf_token,
    };

    HtmlTemplate(template)
}

#[derive(Debug, Deserialize)]
pub struct DonateRequest {
    pub amount_cents: i64,
    pub campaign_slug: Option<String>,
    pub saved_card_id: Option<String>,
    /// Idempotency key from the donate form. See ChargeSavedCardRequest.
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

pub async fn donate_api(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    headers: axum::http::HeaderMap,
    Json(request): Json<DonateRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    let ip = crate::api::state::client_ip(&headers, state.settings.server.trust_forwarded_for());
    if !state.money_limiter.check_and_record(ip) {
        return Err(AppError::TooManyRequests);
    }

    if request.amount_cents <= 0 {
        return Err(AppError::BadRequest("Amount must be positive".to_string()));
    }
    if request.amount_cents > crate::domain::MAX_PAYMENT_CENTS {
        return Err(AppError::BadRequest(format!(
            "Amount exceeds the ${} cap on a single donation",
            crate::domain::MAX_PAYMENT_CENTS / 100,
        )));
    }

    // Resolve the campaign up front. We need both the ID (for the FK
    // we'll write on the Payment row) and the name (for the human-
    // readable description). A blank or unknown slug means "general
    // donation" — recorded but not attributed to any campaign.
    let (campaign_id, campaign_name) = match request.campaign_slug.as_deref() {
        Some(slug) if !slug.is_empty() => {
            match state.service_context.donation_campaign_repo
                .find_by_slug(slug).await?
            {
                // Inactive campaigns aren't accepting donations.
                // Without this filter a member who knows the slug of
                // an archived campaign (cached pages, old emails)
                // could continue to push the campaign's public total
                // up after admins closed it.
                Some(c) if !c.is_active => {
                    return Err(AppError::BadRequest(format!(
                        "Campaign '{}' is no longer accepting donations.",
                        c.name,
                    )));
                }
                Some(c) => (Some(c.id), c.name),
                None => (None, "General donation".to_string()),
            }
        }
        _ => (None, "General donation".to_string()),
    };
    let description = if campaign_id.is_some() {
        format!("Donation to {}", campaign_name)
    } else {
        "General donation".to_string()
    };

    // If saved card provided, charge directly
    if let Some(card_id_str) = &request.saved_card_id {
        let stripe_client = state.stripe_client.as_ref()
            .ok_or_else(|| AppError::ServiceUnavailable("Payment processing not configured".to_string()))?;

        let card_id = Uuid::parse_str(card_id_str)
            .map_err(|_| AppError::BadRequest("Invalid card ID".to_string()))?;

        let card = state.service_context.saved_card_repo
            .find_by_id(card_id)
            .await?
            .ok_or(AppError::NotFound("Card not found".to_string()))?;

        if card.member_id != current_user.member.id {
            return Err(AppError::Forbidden);
        }

        let idempotency_key = request.idempotency_key.clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        // Pending-first: insert local row before charging Stripe, so
        // a successful charge can never end up without a record.
        // The conditional flip below races safely against the
        // payment_intent.succeeded webhook — whoever flips wins.
        let payment_id = Uuid::new_v4();
        let pending = Payment {
            id: payment_id,
            payer: Payer::Member(current_user.member.id),
            amount_cents: request.amount_cents,
            currency: "USD".to_string(),
            status: PaymentStatus::Pending,
            payment_method: PaymentMethod::Stripe,
            external_id: None,
            description: description.clone(),
            kind: PaymentKind::Donation { campaign_id },
            paid_at: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        state.service_context.payment_repo.create(pending).await?;

        let stripe_payment_id = match stripe_client.charge_saved_card(
            current_user.member.id,
            &card.stripe_payment_method_id,
            request.amount_cents,
            &description,
            &idempotency_key,
            payment_id,
        ).await {
            Ok(id) => id,
            Err(e) => {
                let _ = state.service_context.payment_repo.fail_pending_payment(payment_id).await;
                return Err(e);
            }
        };

        // Donation post-work is just the row flip — no dues
        // extension, no rescheduling. So whether we or the webhook
        // wins the flip, the user-visible result is the same.
        let _ = state.service_context.payment_repo
            .complete_pending_payment(payment_id, &stripe_payment_id)
            .await?;

        return Ok((StatusCode::OK, Json(serde_json::json!({
            "payment_id": payment_id,
            "status": "completed",
        }))));
    }

    // No saved card → Stripe Checkout. Use the dedicated donation
    // helper so the webhook handler knows NOT to extend dues — the
    // old code routed donations through the membership helper, which
    // worked only because the webhook's NotFound on slug "donation"
    // caused Stripe-retry-until-idempotent-skip side effects.
    let stripe_client = state.stripe_client.as_ref()
        .ok_or_else(|| AppError::ServiceUnavailable("Payment processing not configured".to_string()))?;

    let (checkout_url, payment_id) = stripe_client.create_donation_checkout_session(
        current_user.member.id,
        &campaign_name,
        campaign_id,
        request.amount_cents,
        format!("{}/portal/payments/success", state.settings.server.base_url),
        format!("{}/portal/payments/cancel", state.settings.server.base_url),
    ).await?;

    Ok((StatusCode::OK, Json(serde_json::json!({
        "payment_id": payment_id,
        "checkout_url": checkout_url,
    }))))
}
