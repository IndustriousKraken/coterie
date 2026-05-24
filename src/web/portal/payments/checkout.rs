use std::sync::Arc;

use axum::{extract::State, http::StatusCode, Extension, Json};
use serde::Deserialize;

use crate::{
    api::{middleware::auth::CurrentUser, state::MoneyLimiter},
    config::Settings,
    error::AppError,
    payments::StripeClient,
    repository::{PaymentRepository, SavedCardRepository},
    service::{
        audit_service::AuditService, billing_service::BillingService,
        membership_type_service::MembershipTypeService,
    },
};

#[derive(Debug, Deserialize)]
pub struct CheckoutRequest {
    pub membership_type_slug: String,
}

#[derive(Debug, Deserialize)]
pub struct ChargeSavedCardRequest {
    pub membership_type_slug: String,
    pub saved_card_id: String,
    /// Idempotency key generated at form-render time (hidden input).
    /// Stable across retries of the same logical payment attempt so
    /// double-clicking "Pay" doesn't double-charge the card.
    /// Optional for API callers; handler falls back to a fresh UUID.
    #[serde(default)]
    pub idempotency_key: Option<String>,
    /// If true, enroll the member in Coterie-managed auto-renewal as
    /// part of this payment: future renewals will charge this saved
    /// card automatically. If false (or omitted), enrollment is left
    /// alone — already-enrolled members stay enrolled (and their
    /// schedule is updated to the new due date), manual members stay
    /// manual.
    #[serde(default)]
    pub enable_auto_renew: Option<bool>,
}

pub async fn checkout_api(
    State(settings): State<Arc<Settings>>,
    State(stripe_client): State<Option<Arc<StripeClient>>>,
    State(membership_type_service): State<Arc<MembershipTypeService>>,
    Extension(current_user): Extension<CurrentUser>,
    Json(request): Json<CheckoutRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    let stripe_client = stripe_client.as_ref().ok_or_else(|| {
        AppError::ServiceUnavailable("Payment processing is not configured".to_string())
    })?;

    let membership_type = membership_type_service
        .get_by_slug(&request.membership_type_slug)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "Membership type '{}' not found",
                request.membership_type_slug
            ))
        })?;

    if !membership_type.is_active {
        return Err(AppError::BadRequest(format!(
            "Membership type '{}' is not currently available",
            membership_type.name
        )));
    }

    let amount_cents = membership_type.fee_cents as i64;

    let (checkout_url, payment_id) = stripe_client
        .create_membership_checkout_session(
            current_user.member.id,
            &membership_type.name,
            &membership_type.slug,
            amount_cents,
            format!("{}/portal/payments/success", settings.server.base_url),
            format!("{}/portal/payments/cancel", settings.server.base_url),
        )
        .await?;

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "payment_id": payment_id,
            "checkout_url": checkout_url,
        })),
    ))
}

/// Charge a saved card for membership dues.
///
/// This handler is the heaviest in the payments file: it needs rate-limit
/// (money_limiter + settings), the Stripe client, the membership-type
/// service, both the payment and saved-card repos, the billing service
/// (for dues extension + auto-renew enrollment), and the audit service.
/// Granular extraction per D1 — even though we cross the D3 threshold,
/// the dependencies are individually meaningful and the signature is
/// the documentation a reader looks for.
pub async fn charge_saved_card_api(
    State(settings): State<Arc<Settings>>,
    State(money_limiter): State<MoneyLimiter>,
    State(stripe_client): State<Option<Arc<StripeClient>>>,
    State(membership_type_service): State<Arc<MembershipTypeService>>,
    State(payment_repo): State<Arc<dyn PaymentRepository>>,
    State(saved_card_repo): State<Arc<dyn SavedCardRepository>>,
    State(billing_service): State<Arc<BillingService>>,
    State(audit_service): State<Arc<AuditService>>,
    Extension(current_user): Extension<CurrentUser>,
    headers: axum::http::HeaderMap,
    Json(request): Json<ChargeSavedCardRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    let ip = crate::api::state::client_ip(&headers, settings.server.trust_forwarded_for());
    if !money_limiter.0.check_and_record(ip) {
        return Err(AppError::TooManyRequests);
    }

    let stripe_client = stripe_client.as_ref().ok_or_else(|| {
        AppError::ServiceUnavailable("Payment processing not configured".to_string())
    })?;

    let membership_type = membership_type_service
        .get_by_slug(&request.membership_type_slug)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "Membership type '{}' not found",
                request.membership_type_slug
            ))
        })?;

    if !membership_type.is_active {
        return Err(AppError::BadRequest(format!(
            "Membership type '{}' is not currently available",
            membership_type.name
        )));
    }

    // Verify the card belongs to this user
    let card_id = uuid::Uuid::parse_str(&request.saved_card_id)
        .map_err(|_| AppError::BadRequest("Invalid card ID".to_string()))?;

    let card = saved_card_repo
        .find_by_id(card_id)
        .await?
        .ok_or(AppError::NotFound("Card not found".to_string()))?;

    if card.member_id != current_user.member.id {
        return Err(AppError::Forbidden);
    }

    let amount_cents = membership_type.fee_cents as i64;
    let description = format!("{} Membership Payment", membership_type.name);

    // Idempotency key: use the one from the form if present (stable across
    // double-submits), otherwise generate a fresh UUID. Callers that care
    // about double-charge protection should always send a key.
    let idempotency_key = request
        .idempotency_key
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    // Pending-first pattern: insert the local Payment row BEFORE
    // calling Stripe. If Stripe charges but the local insert had
    // failed, we'd have a charge with no record. Going Pending →
    // Completed via a conditional UPDATE also gives us a race-free
    // hand-off with the payment_intent.succeeded webhook: whoever
    // flips the status owns the post-payment work below.
    let payment_id = uuid::Uuid::new_v4();
    let pending = crate::domain::Payment {
        id: payment_id,
        payer: crate::domain::Payer::Member(current_user.member.id),
        amount_cents,
        currency: "USD".to_string(),
        status: crate::domain::PaymentStatus::Pending,
        payment_method: crate::domain::PaymentMethod::Stripe,
        external_id: None,
        description: description.clone(),
        kind: crate::domain::PaymentKind::Membership,
        paid_at: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    payment_repo.create(pending).await?;

    // Charge the card. On error, flip the Pending row to Failed so
    // it doesn't haunt the "Pending older than 5 minutes — investigate"
    // queue (and so the webhook self-heal won't act on a dead row).
    let stripe_payment_id = match stripe_client
        .charge_saved_card(
            current_user.member.id,
            &card.stripe_payment_method_id,
            amount_cents,
            &description,
            &idempotency_key,
            payment_id,
        )
        .await
    {
        Ok(id) => id,
        Err(e) => {
            let _ = payment_repo.fail_pending_payment(payment_id).await;
            return Err(e);
        }
    };

    // Race-free flip. If we win (won_flip=true), do the post-work.
    // If the webhook beat us, it already did the post-work and we
    // return success without duplicating dues extension.
    let won_flip = payment_repo
        .complete_pending_payment(payment_id, &stripe_payment_id)
        .await?;
    if !won_flip {
        return Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "payment_id": payment_id,
                "status": "completed",
            })),
        ));
    }

    // Both the dues-extension and the auto-renew branch below run
    // through the shared BillingService.
    let billing_service = billing_service.as_ref();

    // Extend dues first so the new dues_paid_until is what auto-renew
    // schedules off of. This is the source of truth for "when does the
    // next renewal fire" — if scheduling reads the old date, the queued
    // charge would fire on the previous cycle's day.
    billing_service
        .auto_renew
        .extend_member_dues_by_slug(payment_id, current_user.member.id, &membership_type.slug)
        .await?;

    // Branch on auto-renew intent:
    //   - enable_auto_renew=true: enroll (or re-enroll) and schedule.
    //   - already enrolled: keep them on the loop, refresh the schedule
    //     so it points at the new dues_paid_until.
    //   - otherwise: this is a one-time pay-through, leave billing_mode
    //     alone and don't queue a future charge.
    //
    // Failures are logged but don't roll back the successful charge —
    // the member's dues are paid; the worst case is they fall off the
    // auto-renew loop and an operator has to re-enroll them.
    let opt_in = request.enable_auto_renew.unwrap_or(false);
    if opt_in {
        if let Err(e) = billing_service
            .auto_renew
            .enable_auto_renew(current_user.member.id, &request.membership_type_slug)
            .await
        {
            tracing::error!(
                "Charged member {} but failed to enable auto-renew: {}",
                current_user.member.id,
                e,
            );
        } else {
            audit_service
                .log(
                    Some(current_user.member.id),
                    "enable_auto_renew",
                    "member",
                    &current_user.member.id.to_string(),
                    None,
                    Some(&format!(
                        "via saved card payment ({})",
                        request.membership_type_slug
                    )),
                    None,
                )
                .await;
        }
    } else if let Err(e) = billing_service
        .auto_renew
        .reschedule_after_payment(current_user.member.id, &request.membership_type_slug)
        .await
    {
        tracing::error!(
            "Charged member {} but failed to reschedule auto-renew: {}",
            current_user.member.id,
            e,
        );
    }

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "payment_id": payment_id,
            "status": "completed",
        })),
    ))
}
