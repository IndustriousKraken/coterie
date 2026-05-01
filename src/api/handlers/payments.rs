//! JSON payment endpoints. Narrowed to:
//!   - the inbound Stripe webhook (`stripe_webhook`),
//!   - the saved-card management endpoints the portal frontend
//!     `fetch()`-es directly because Stripe.js needs JSON in / JSON out.
//!
//! All admin-side payment recording lives in `PaymentService` and is
//! reachable via the portal admin UI; the previous JSON `create_manual`
//! / `waive` endpoints were deleted because the portal doesn't use
//! them and external callers shouldn't either (admin actions belong
//! inside Coterie).

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
    Extension,
    response::IntoResponse,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    api::{state::AppState, middleware::auth::CurrentUser},
    domain::SavedCard,
    error::{AppError, Result},
};


pub async fn stripe_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<impl IntoResponse> {
    let dispatcher = match state.webhook_dispatcher.as_ref() {
        Some(d) => d,
        None => return Ok(StatusCode::SERVICE_UNAVAILABLE),
    };

    // Get Stripe signature from headers
    let stripe_signature = headers
        .get("stripe-signature")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::BadRequest("Missing Stripe signature".to_string()))?;

    // The webhook handler needs BillingService to re-schedule auto-renew
    // charges when an enrolled member pays early via Checkout (otherwise
    // the queued ScheduledPayment fires at the wrong time and double-
    // charges them).
    let billing_service = state.billing_service.as_ref();

    // Handle the webhook. On signature failure, dispatch an admin
    // alert before returning the error so an operator gets notified
    // (in Discord, if configured) — bad signature usually means
    // either Stripe rotated the webhook secret and we still have
    // the old one, OR something is forging requests at our endpoint.
    if let Err(e) = dispatcher.handle_webhook(&body, stripe_signature, &billing_service).await {
        if matches!(&e, AppError::BadRequest(msg) if msg.contains("Invalid signature")) {
            state.service_context.integration_manager
                .handle_event(crate::integrations::IntegrationEvent::AdminAlert {
                    subject: "Stripe webhook signature failed".to_string(),
                    body: format!(
                        "A Stripe webhook arrived with an invalid signature. \
                         If you recently rotated the webhook secret in Stripe, \
                         update it in /portal/admin/settings (it lives in env \
                         config currently — see deploy/.env). If you didn't \
                         rotate anything, someone may be forging webhooks at \
                         /api/payments/webhook/stripe."
                    ),
                })
                .await;
        } else if matches!(&e, AppError::BadRequest(msg) if msg.contains("clock drift")) {
            // Without this alert, a >5min server clock skew would
            // silently reject every Stripe webhook (signature is
            // tied to the timestamp). Members would pay successfully
            // but dues / refunds wouldn't update on our side until
            // someone notices.
            state.service_context.integration_manager
                .handle_event(crate::integrations::IntegrationEvent::AdminAlert {
                    subject: "Stripe webhook rejected — clock drift".to_string(),
                    body: format!(
                        "A Stripe webhook was rejected because the server's \
                         clock has drifted more than Stripe's tolerance (~5 \
                         min) from real time. Until this is fixed, EVERY \
                         webhook will fail and payments will not be \
                         processed. Check NTP / time sync on the host. \
                         Error detail: {}",
                        e,
                    ),
                })
                .await;
        }
        return Err(e);
    }

    Ok(StatusCode::OK)
}


// ============================================================
// Saved Card (Payment Method) Handlers
// ============================================================

#[derive(Debug, Serialize)]
pub struct SetupIntentResponse {
    pub client_secret: String,
}

#[derive(Debug, Serialize)]
pub struct SavedCardResponse {
    pub id: Uuid,
    pub display_name: String,
    pub exp_display: String,
    pub is_default: bool,
    pub is_expired: bool,
}

impl From<SavedCard> for SavedCardResponse {
    fn from(card: SavedCard) -> Self {
        SavedCardResponse {
            id: card.id,
            display_name: card.display_name(),
            exp_display: card.exp_display(),
            is_default: card.is_default,
            is_expired: card.is_expired(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SaveCardRequest {
    pub stripe_payment_method_id: String,
    pub set_as_default: Option<bool>,
}

/// Create a SetupIntent for adding a new payment method
pub async fn create_setup_intent(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<SetupIntentResponse>> {
    let stripe_client = state.stripe_client.as_ref()
        .ok_or_else(|| AppError::ServiceUnavailable("Payment processing not configured".to_string()))?;

    let client_secret = stripe_client.create_setup_intent(
        user.member.id,
        &user.member.email,
        &user.member.full_name,
    ).await?;

    Ok(Json(SetupIntentResponse { client_secret }))
}

/// Save a payment method after SetupIntent succeeds
pub async fn save_card(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Json(request): Json<SaveCardRequest>,
) -> Result<(StatusCode, Json<SavedCardResponse>)> {
    let stripe_client = state.stripe_client.as_ref()
        .ok_or_else(|| AppError::ServiceUnavailable("Payment processing not configured".to_string()))?;

    // Get card details from Stripe
    let card_details = stripe_client.get_payment_method_details(&request.stripe_payment_method_id).await?;

    // Cross-member PM stapling guard. Stripe's PaymentMethod attaches
    // to exactly one Customer; a PM ID belonging to another member's
    // Customer would let the requester surface that card's last4 +
    // brand in their own saved-cards list. Refuse unless the PM
    // belongs to THIS member's Stripe Customer (or is unattached,
    // which is the normal SetupIntent state — confirmCardSetup
    // attaches it during this flow).
    let member_customer_id = user.member.stripe_customer_id.as_deref();
    match (card_details.customer_id.as_deref(), member_customer_id) {
        (None, _) => { /* fresh SetupIntent PM, attached to this member's customer momentarily */ }
        (Some(pm_cust), Some(my_cust)) if pm_cust == my_cust => { /* match */ }
        _ => return Err(AppError::Forbidden),
    }

    // Check if this is the first card (will be default)
    let existing_cards = state.service_context.saved_card_repo.find_by_member(user.member.id).await?;
    let is_default = existing_cards.is_empty() || request.set_as_default.unwrap_or(false);

    // Create the saved card record
    let card = SavedCard {
        id: Uuid::new_v4(),
        member_id: user.member.id,
        stripe_payment_method_id: request.stripe_payment_method_id,
        card_last_four: card_details.last_four,
        card_brand: card_details.brand,
        exp_month: card_details.exp_month,
        exp_year: card_details.exp_year,
        is_default,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let card = state.service_context.saved_card_repo.create(card).await?;

    // If this card is default and there were existing cards, clear other defaults
    if is_default && !existing_cards.is_empty() {
        state.service_context.saved_card_repo.set_default(user.member.id, card.id).await?;
    }

    // If the member is on a Stripe-managed subscription, this card
    // save is the trigger to migrate them to Coterie-managed
    // auto-renew. Best-effort: log on failure but don't bounce the
    // save itself — the card IS in Coterie's table either way, and
    // an admin can finish the migration manually if needed.
    if user.member.billing_mode == crate::domain::BillingMode::StripeSubscription {
        match state.billing_service.migrate_to_coterie_managed(user.member.id).await {
            Ok(true) => {
                state.service_context.audit_service.log(
                    Some(user.member.id),
                    "migrate_stripe_to_coterie",
                    "member",
                    &user.member.id.to_string(),
                    None,
                    Some("triggered by save_card"),
                    None,
                ).await;
            }
            Ok(false) => {} // Wasn't actually on stripe_sub by the time we ran; harmless.
            Err(e) => {
                tracing::error!(
                    "Card saved for member {} but stripe→coterie migration failed: {}",
                    user.member.id, e,
                );
            }
        }
    }

    Ok((StatusCode::CREATED, Json(card.into())))
}

/// List all saved cards for the current user
pub async fn list_saved_cards(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<Vec<SavedCardResponse>>> {
    let cards = state.service_context.saved_card_repo.find_by_member(user.member.id).await?;
    let responses: Vec<SavedCardResponse> = cards.into_iter().map(Into::into).collect();
    Ok(Json(responses))
}

/// Delete a saved card
pub async fn delete_saved_card(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Path(card_id): Path<Uuid>,
) -> Result<StatusCode> {
    // Verify the card belongs to this user
    let card = state.service_context.saved_card_repo.find_by_id(card_id).await?
        .ok_or(AppError::NotFound("Card not found".to_string()))?;

    if card.member_id != user.member.id {
        return Err(AppError::Forbidden);
    }

    state.service_context.saved_card_repo.delete(card_id).await?;

    // Also detach on Stripe's side. Local delete makes the row
    // invisible to Coterie, but without detach the PaymentMethod
    // continues to exist on the Stripe Customer indefinitely. Best-
    // effort — a Stripe failure shouldn't fail the user-visible
    // delete, but we log it loudly so operators can clean up.
    if let Some(stripe) = state.stripe_client.as_ref() {
        if let Err(e) = stripe.detach_payment_method(&card.stripe_payment_method_id).await {
            tracing::error!(
                "Locally deleted card {} (pm={}) but Stripe detach failed: {}",
                card_id, card.stripe_payment_method_id, e,
            );
        }
    }

    Ok(StatusCode::NO_CONTENT)
}

/// Set a card as the default payment method
pub async fn set_default_card(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Path(card_id): Path<Uuid>,
) -> Result<StatusCode> {
    // Verify the card belongs to this user
    let card = state.service_context.saved_card_repo.find_by_id(card_id).await?
        .ok_or(AppError::NotFound("Card not found".to_string()))?;

    if card.member_id != user.member.id {
        return Err(AppError::Forbidden);
    }

    state.service_context.saved_card_repo.set_default(user.member.id, card_id).await?;
    Ok(StatusCode::OK)
}
