//! Public donation API — the unauthenticated, money-moving
//! `POST /public/donate`.

use std::sync::Arc;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    api::{middleware::bot_challenge::BotChallengeVerifier, state::MoneyLimiter},
    config::Settings,
    error::{AppError, Result},
    payments::StripeClient,
    repository::{DonationCampaignRepository, MemberRepository},
};

#[derive(Debug, Deserialize, ToSchema)]
pub struct PublicDonateRequest {
    pub amount_cents: i64,
    pub email: String,
    pub name: String,
    /// Optional campaign slug. If absent or empty, the donation is
    /// recorded as a general donation (no campaign attribution).
    #[serde(default)]
    pub campaign_slug: Option<String>,
    /// Bot-challenge token from the marketing site's CAPTCHA widget.
    /// Required when the org has configured a provider; ignored when
    /// `bot_challenge.provider = "disabled"`. See the `bot_challenge`
    /// settings (migration 041) and `DynamicBotChallengeVerifier`.
    #[serde(default)]
    pub captcha_token: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PublicDonateResponse {
    pub payment_id: Uuid,
    /// Stripe-hosted Checkout URL. The frontend redirects the donor here.
    pub checkout_url: String,
}

/// POST /public/donate — accepts a donation from a non-authenticated
/// donor and returns a Stripe Checkout URL to redirect them to.
///
/// Flow:
///   1. Validate amount + email + name + campaign-if-given
///   2. Check IP rate limit (money_limiter, 10/min/IP)
///   3. If donor's email matches an existing member → attach donation
///      to that member's payment history. Otherwise → record as a
///      public donation with donor_name + donor_email on the row.
///   4. Create Stripe Checkout session, return URL.
///   5. (Webhook side) When the session completes, the existing
///      payment_intent.succeeded / checkout.session.completed handlers
///      flip the row to Completed. Donations don't extend dues, so
///      there's no further bookkeeping.
///
/// CORS: same origin policy as other /public/* endpoints. The public
/// site (e.g. neontemple.net) is expected to be in
/// COTERIE__SERVER__CORS_ORIGINS.
#[utoipa::path(
    post,
    path = "/public/donate",
    tag = "public",
    request_body = PublicDonateRequest,
    responses(
        (status = 200, description = "Stripe Checkout session created; redirect donor to checkout_url",
            body = PublicDonateResponse),
        (status = 400, description = "Invalid amount, email, name, or campaign"),
        (status = 429, description = "Rate-limit hit (per-IP money limiter)"),
        (status = 503, description = "Payment processing not configured"),
    ),
)]
pub async fn donate(
    State(settings): State<Arc<Settings>>,
    State(money_limiter): State<MoneyLimiter>,
    State(bot_challenge_verifier): State<Arc<dyn BotChallengeVerifier>>,
    State(donation_campaign_repo): State<Arc<dyn DonationCampaignRepository>>,
    State(stripe_client): State<Option<Arc<StripeClient>>>,
    State(member_repo): State<Arc<dyn MemberRepository>>,
    headers: HeaderMap,
    Json(request): Json<PublicDonateRequest>,
) -> Result<(StatusCode, Json<PublicDonateResponse>)> {
    // Rate limit by client IP. Public endpoint with payment side-effects
    // is the prime card-testing target — the limiter caps each IP at
    // 10 attempts per minute. Rate limit BEFORE the bot challenge so
    // a bursting IP can't burn through the provider's quota.
    let ip = crate::api::state::client_ip(&headers, settings.server.trust_forwarded_for());
    if !money_limiter.0.check_and_record(ip, "public.donate") {
        return Err(AppError::TooManyRequests);
    }

    // Bot-challenge verification BEFORE Stripe Checkout. Carding
    // attacks abuse this endpoint to test stolen cards; the per-IP
    // limiter stops single-source bursts but distributed bots roll
    // right past it. Fail closed when a provider is configured.
    if bot_challenge_verifier
        .verify("public/donate", request.captcha_token.as_deref(), Some(ip))
        .await
        .is_err()
    {
        return Err(AppError::Forbidden);
    }

    // Validation. Bounds match the logged-in donate flow.
    if request.amount_cents <= 0 {
        return Err(AppError::BadRequest("Amount must be positive".to_string()));
    }
    if request.amount_cents > crate::domain::MAX_PAYMENT_CENTS {
        return Err(AppError::BadRequest(format!(
            "Amount exceeds the ${} cap on a single donation",
            crate::domain::MAX_PAYMENT_CENTS / 100,
        )));
    }
    let email = request.email.trim();
    let name = request.name.trim();
    if email.is_empty() || !email.contains('@') {
        return Err(AppError::BadRequest("Valid email is required".to_string()));
    }
    if email.len() > 254 {
        return Err(AppError::BadRequest("Email too long".to_string()));
    }
    if name.is_empty() {
        return Err(AppError::BadRequest("Name is required".to_string()));
    }
    if name.len() > 200 {
        return Err(AppError::BadRequest("Name too long".to_string()));
    }

    // Resolve campaign. Same logic as the logged-in path: blank/missing
    // slug = general donation; unknown slug also = general (donor
    // shouldn't get a hard error for stale URL); inactive = reject.
    let (campaign_id, campaign_name) = match request.campaign_slug.as_deref() {
        Some(slug) if !slug.is_empty() => match donation_campaign_repo.find_by_slug(slug).await? {
            Some(c) if !c.is_active => {
                return Err(AppError::BadRequest(format!(
                    "Campaign '{}' is no longer accepting donations.",
                    c.name,
                )));
            }
            Some(c) => (Some(c.id), c.name),
            None => (None, "General donation".to_string()),
        },
        _ => (None, "General donation".to_string()),
    };

    // Email match → existing member? If yes, route through the
    // member-attributed donation flow so the donation appears in their
    // payment history. If no, public-donation flow with donor identity
    // captured on the payment row directly.
    let stripe_client = stripe_client.as_ref().ok_or_else(|| {
        AppError::ServiceUnavailable("Payment processing not configured".to_string())
    })?;

    let success_url = format!("{}/portal/payments/success", settings.server.base_url);
    let cancel_url = format!("{}/portal/payments/cancel", settings.server.base_url);

    let existing_member = member_repo.find_by_email(email).await?;

    let (checkout_url, payment_id) = match existing_member {
        Some(member) => {
            stripe_client
                .create_donation_checkout_session(
                    member.id,
                    &campaign_name,
                    campaign_id,
                    request.amount_cents,
                    success_url,
                    cancel_url,
                )
                .await?
        }
        None => {
            stripe_client
                .create_public_donation_checkout_session(
                    name,
                    email,
                    &campaign_name,
                    campaign_id,
                    request.amount_cents,
                    success_url,
                    cancel_url,
                )
                .await?
        }
    };

    Ok((
        StatusCode::OK,
        Json(PublicDonateResponse {
            payment_id,
            checkout_url,
        }),
    ))
}
