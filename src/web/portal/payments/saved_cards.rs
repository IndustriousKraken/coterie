use std::sync::Arc;

use askama::Template;
use axum::{extract::State, http::StatusCode, response::IntoResponse, Extension, Json};
use serde::Deserialize;

use crate::{
    api::{
        middleware::auth::{CurrentUser, SessionInfo},
        state::MoneyLimiter,
    },
    auth::CsrfService,
    config::Settings,
    error::AppError,
    integrations::IntegrationManager,
    payments::StripeClient,
    repository::SavedCardRepository,
    service::{
        audit_service::AuditService, billing_service::BillingService,
        membership_type_service::MembershipTypeService,
    },
    web::templates::{BaseContext, HtmlTemplate},
};

#[derive(Template)]
#[template(path = "portal/payment_methods.html")]
pub struct PaymentMethodsTemplate {
    pub base: BaseContext,
    pub stripe_enabled: bool,
    pub stripe_publishable_key: String,
    /// True when the member is on Coterie-managed auto-renew. Drives
    /// the "Auto-renew" toggle UI on this page — enrolled members see
    /// a "Turn off" button, unenrolled members see a "Turn on" button
    /// (only when they have a default card).
    pub is_auto_renew: bool,
    /// True when the member is on a legacy Stripe-managed subscription
    /// (grandfathered from before Coterie owned billing). They're on
    /// auto-renew, but it's Stripe controlling the cycle. UI surfaces
    /// a "Switch to Coterie auto-renew" call to action.
    pub is_stripe_subscription: bool,
    /// Whether the member has at least one default saved card. Used
    /// to disable the "Turn on auto-renew" button when they don't —
    /// a scheduled charge against no card would just fail.
    pub has_default_card: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAutoRenewRequest {
    pub enable: bool,
}

pub async fn payment_methods_page(
    State(csrf_service): State<Arc<CsrfService>>,
    State(settings): State<Arc<Settings>>,
    State(stripe_client): State<Option<Arc<StripeClient>>>,
    State(saved_card_repo): State<Arc<dyn SavedCardRepository>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> impl IntoResponse {
    let base = BaseContext::for_member(&csrf_service, &current_user, &session_info).await;

    let stripe_enabled = stripe_client.is_some();
    let stripe_publishable_key = settings.stripe.publishable_key.clone().unwrap_or_default();

    let is_auto_renew =
        current_user.member.billing_mode == crate::domain::BillingMode::CoterieManaged;
    let is_stripe_subscription =
        current_user.member.billing_mode == crate::domain::BillingMode::StripeSubscription;

    let has_default_card = saved_card_repo
        .find_default_for_member(current_user.member.id)
        .await
        .ok()
        .flatten()
        .is_some();

    let template = PaymentMethodsTemplate {
        base,
        stripe_enabled,
        stripe_publishable_key,
        is_auto_renew,
        is_stripe_subscription,
        has_default_card,
    };

    HtmlTemplate(template)
}

/// Toggle Coterie-managed auto-renew on/off from the payment methods
/// page. Disabling cancels any pending scheduled payments and flips
/// `billing_mode` back to `manual`. Enabling looks up the member's
/// current membership type to know which billing period to schedule
/// against.
///
/// Idempotent: enabling an already-enrolled member just refreshes the
/// schedule; disabling a manual member is a no-op.
pub async fn update_auto_renew_api(
    State(settings): State<Arc<Settings>>,
    State(money_limiter): State<MoneyLimiter>,
    State(billing_service): State<Arc<BillingService>>,
    State(membership_type_service): State<Arc<MembershipTypeService>>,
    State(saved_card_repo): State<Arc<dyn SavedCardRepository>>,
    State(audit_service): State<Arc<AuditService>>,
    Extension(current_user): Extension<CurrentUser>,
    headers: axum::http::HeaderMap,
    Json(request): Json<UpdateAutoRenewRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let ip = crate::api::state::client_ip(&headers, settings.server.trust_forwarded_for());
    if !money_limiter.0.check_and_record(ip) {
        return Err(AppError::TooManyRequests);
    }

    let billing_service = billing_service.as_ref();

    if request.enable {
        // Need the member's current membership type slug to schedule
        // a renewal — that's where billing period (monthly/yearly) and
        // amount come from.
        let mt = membership_type_service
            .get(current_user.member.membership_type_id)
            .await?
            .ok_or_else(|| {
                AppError::Internal(
                    "Member's membership type was deleted; contact an admin.".to_string(),
                )
            })?;

        // Block enabling without a default card UNLESS the member
        // is on a Stripe-managed subscription — in that case the
        // migration will import their cards from Stripe before the
        // first scheduled charge runs, so requiring a Coterie-side
        // card up front would create a chicken-and-egg problem.
        let on_stripe_sub =
            current_user.member.billing_mode == crate::domain::BillingMode::StripeSubscription;
        if !on_stripe_sub {
            let default_card = saved_card_repo
                .find_default_for_member(current_user.member.id)
                .await?;
            if default_card.is_none() {
                return Err(AppError::BadRequest(
                    "Save a default card before enabling auto-renew.".to_string(),
                ));
            }
        }

        billing_service
            .auto_renew
            .enable_auto_renew(current_user.member.id, &mt.slug)
            .await?;

        audit_service
            .log(
                Some(current_user.member.id),
                "enable_auto_renew",
                "member",
                &current_user.member.id.to_string(),
                None,
                Some("via payment methods page"),
                None,
            )
            .await;
    } else {
        billing_service
            .auto_renew
            .disable_auto_renew(current_user.member.id)
            .await?;

        audit_service
            .log(
                Some(current_user.member.id),
                "disable_auto_renew",
                "member",
                &current_user.member.id.to_string(),
                None,
                Some("via payment methods page"),
                None,
            )
            .await;
    }

    Ok(Json(serde_json::json!({
        "is_auto_renew": request.enable,
    })))
}

/// HTMX endpoint - list saved cards as HTML
pub async fn saved_cards_html_api(
    State(saved_card_repo): State<Arc<dyn SavedCardRepository>>,
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {
    let cards = saved_card_repo
        .find_by_member(current_user.member.id)
        .await
        .unwrap_or_default();

    let rows = cards
        .iter()
        .map(crate::web::portal::partials::saved_card_row_from)
        .collect();
    crate::web::portal::partials::saved_card_list(rows)
}

/// HTMX endpoint - delete a saved card
pub async fn delete_card_api(
    State(saved_card_repo): State<Arc<dyn SavedCardRepository>>,
    State(stripe_client): State<Option<Arc<StripeClient>>>,
    State(integration_manager): State<Arc<IntegrationManager>>,
    Extension(current_user): Extension<CurrentUser>,
    axum::extract::Path(card_id): axum::extract::Path<uuid::Uuid>,
) -> Result<StatusCode, AppError> {
    let card = saved_card_repo
        .find_by_id(card_id)
        .await?
        .ok_or(AppError::NotFound("Card not found".to_string()))?;

    if card.member_id != current_user.member.id {
        return Err(AppError::Forbidden);
    }

    // For coterie_managed members removing their last card, the
    // hourly billing runner will fail to charge their next renewal
    // without a card to draw on. Surface the situation rather than
    // letting the failure manifest silently weeks later.
    let other_cards = saved_card_repo
        .find_by_member(current_user.member.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|c| c.id != card_id)
        .count();
    let leaving_with_no_cards = other_cards == 0
        && current_user.member.billing_mode == crate::domain::BillingMode::CoterieManaged;

    saved_card_repo.delete(card_id).await?;

    // Detach on Stripe so the card doesn't outlive the Coterie row.
    if let Some(stripe) = stripe_client.as_ref() {
        if let Err(e) = stripe
            .detach_payment_method(&card.stripe_payment_method_id)
            .await
        {
            tracing::error!(
                "Locally deleted card {} (pm={}) but Stripe detach failed: {}",
                card_id,
                card.stripe_payment_method_id,
                e,
            );
        }
    }

    if leaving_with_no_cards {
        integration_manager
            .handle_event(crate::integrations::IntegrationEvent::AdminAlert {
                subject: format!(
                    "Auto-renew member {} deleted their last card",
                    current_user.member.id,
                ),
                body: format!(
                    "Member {} <{}> is on coterie_managed auto-renew but \
                     just removed their only saved card. The next \
                     scheduled charge will fail. Reach out to confirm \
                     they meant to disable auto-renew, or have them \
                     re-add a card.",
                    current_user.member.full_name, current_user.member.email,
                ),
            })
            .await;
    }

    Ok(StatusCode::OK)
}

/// HTMX endpoint - set a card as default
pub async fn set_default_card_api(
    State(saved_card_repo): State<Arc<dyn SavedCardRepository>>,
    Extension(current_user): Extension<CurrentUser>,
    axum::extract::Path(card_id): axum::extract::Path<uuid::Uuid>,
) -> Result<StatusCode, AppError> {
    let card = saved_card_repo
        .find_by_id(card_id)
        .await?
        .ok_or(AppError::NotFound("Card not found".to_string()))?;

    if card.member_id != current_user.member.id {
        return Err(AppError::Forbidden);
    }

    saved_card_repo
        .set_default(current_user.member.id, card_id)
        .await?;
    Ok(StatusCode::OK)
}
