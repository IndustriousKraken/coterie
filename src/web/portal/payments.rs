use askama::Template;
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Extension,
    Json,
};
use serde::Deserialize;

use crate::{
    api::{
        middleware::auth::{CurrentUser, SessionInfo},
        state::AppState,
    },
    error::AppError,
    web::templates::{BaseContext, HtmlTemplate},
};

#[derive(Template)]
#[template(path = "portal/payments.html")]
pub struct PaymentsTemplate {
    pub base: BaseContext,
}

#[derive(Template)]
#[template(path = "portal/payment_new.html")]
pub struct PaymentNewTemplate {
    pub base: BaseContext,
    pub stripe_enabled: bool,
    pub membership_types: Vec<MembershipTypeDisplay>,
    pub saved_cards: Vec<SavedCardDisplay>,
    pub stripe_publishable_key: String,
    /// True when the member is already on Coterie-managed auto-renew.
    /// We hide the "Enable auto-renew" checkbox in that case and show
    /// an "already enrolled" badge instead — the saved-card payment
    /// will keep them enrolled and refresh the schedule automatically.
    pub is_auto_renew: bool,
}

pub struct SavedCardDisplay {
    pub id: String,
    pub display_name: String,
    pub exp_display: String,
    pub is_default: bool,
}

pub struct MembershipTypeDisplay {
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub color: Option<String>,
    pub fee_display: String,
    pub billing_period: String,
}

#[derive(Template)]
#[template(path = "portal/payment_success.html")]
pub struct PaymentSuccessTemplate {
    pub base: BaseContext,
}

#[derive(Template)]
#[template(path = "portal/payment_cancel.html")]
pub struct PaymentCancelTemplate {
    pub base: BaseContext,
}

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

pub async fn payments_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session): Extension<SessionInfo>,
) -> impl IntoResponse {
    let template = PaymentsTemplate {
        base: BaseContext::for_member(&state, &current_user, &session).await,
    };

    HtmlTemplate(template)
}

// API endpoint for full payments list
pub async fn payments_list_api(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {
    let payments = state.service_context.payment_repo
        .find_by_member(current_user.member.id)
        .await
        .unwrap_or_default();

    let rows = payments.iter()
        .map(crate::web::portal::partials::member_payment_row_from)
        .collect();
    crate::web::portal::partials::member_payment_list(rows)
}

// API endpoint for payments summary
pub async fn payments_summary_api(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {
    use crate::domain::PaymentStatus;

    let payments = state.service_context.payment_repo
        .find_by_member(current_user.member.id)
        .await
        .unwrap_or_default();

    let total: i64 = payments.iter()
        .filter(|p| p.status == PaymentStatus::Completed)
        .map(|p| p.amount_cents)
        .sum();

    axum::response::Html(format!("${:.2}", total as f64 / 100.0))
}

// API endpoint for dues status
pub async fn dues_status_api(
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {
    let status: &'static str = if let Some(dues_until) = current_user.member.dues_paid_until {
        if dues_until > chrono::Utc::now() { "current" } else { "expired" }
    } else {
        "unpaid"
    };
    crate::web::portal::partials::dues_status_pill(status)
}

// API endpoint for next due date
pub async fn next_due_api(
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {
    let next_due = if let Some(dues_until) = current_user.member.dues_paid_until {
        dues_until.format("%B %d, %Y").to_string()
    } else {
        "—".to_string()
    };

    axum::response::Html(next_due)
}

pub async fn payment_success_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session): Extension<SessionInfo>,
) -> impl IntoResponse {
    let template = PaymentSuccessTemplate {
        base: BaseContext::for_member(&state, &current_user, &session).await,
    };

    HtmlTemplate(template)
}

pub async fn payment_cancel_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session): Extension<SessionInfo>,
) -> impl IntoResponse {
    let template = PaymentCancelTemplate {
        base: BaseContext::for_member(&state, &current_user, &session).await,
    };

    HtmlTemplate(template)
}

pub async fn payment_new_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> impl IntoResponse {
    let base = BaseContext::for_member(&state, &current_user, &session_info).await;

    let stripe_enabled = state.stripe_client.is_some();
    let stripe_publishable_key = state.settings.stripe.publishable_key
        .clone()
        .unwrap_or_default();

    let membership_types = state.service_context.membership_type_service
        .list(false)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|mt| MembershipTypeDisplay {
            name: mt.name,
            slug: mt.slug,
            description: mt.description,
            color: mt.color,
            fee_display: format!("{:.2}", mt.fee_cents as f64 / 100.0),
            billing_period: mt.billing_period,
        })
        .collect();

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

    let is_auto_renew = current_user.member.billing_mode
        == crate::domain::BillingMode::CoterieManaged;

    let template = PaymentNewTemplate {
        base,
        stripe_enabled,
        membership_types,
        saved_cards,
        stripe_publishable_key,
        is_auto_renew,
    };

    HtmlTemplate(template)
}

pub async fn checkout_api(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Json(request): Json<CheckoutRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    let stripe_client = state.stripe_client.as_ref()
        .ok_or_else(|| AppError::ServiceUnavailable("Payment processing is not configured".to_string()))?;

    let membership_type = state.service_context.membership_type_service
        .get_by_slug(&request.membership_type_slug)
        .await?
        .ok_or_else(|| AppError::NotFound(format!(
            "Membership type '{}' not found", request.membership_type_slug
        )))?;

    if !membership_type.is_active {
        return Err(AppError::BadRequest(format!(
            "Membership type '{}' is not currently available", membership_type.name
        )));
    }

    let amount_cents = membership_type.fee_cents as i64;

    let (checkout_url, payment_id) = stripe_client.create_membership_checkout_session(
        current_user.member.id,
        &membership_type.name,
        &membership_type.slug,
        amount_cents,
        format!("{}/portal/payments/success", state.settings.server.base_url),
        format!("{}/portal/payments/cancel", state.settings.server.base_url),
    ).await?;

    Ok((StatusCode::OK, Json(serde_json::json!({
        "payment_id": payment_id,
        "checkout_url": checkout_url,
    }))))
}

/// Charge a saved card for membership dues
pub async fn charge_saved_card_api(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    headers: axum::http::HeaderMap,
    Json(request): Json<ChargeSavedCardRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
    let ip = crate::api::state::client_ip(&headers, state.settings.server.trust_forwarded_for());
    if !state.money_limiter.check_and_record(ip) {
        return Err(AppError::TooManyRequests);
    }

    let stripe_client = state.stripe_client.as_ref()
        .ok_or_else(|| AppError::ServiceUnavailable("Payment processing not configured".to_string()))?;

    let membership_type = state.service_context.membership_type_service
        .get_by_slug(&request.membership_type_slug)
        .await?
        .ok_or_else(|| AppError::NotFound(format!(
            "Membership type '{}' not found", request.membership_type_slug
        )))?;

    if !membership_type.is_active {
        return Err(AppError::BadRequest(format!(
            "Membership type '{}' is not currently available", membership_type.name
        )));
    }

    // Verify the card belongs to this user
    let card_id = uuid::Uuid::parse_str(&request.saved_card_id)
        .map_err(|_| AppError::BadRequest("Invalid card ID".to_string()))?;

    let card = state.service_context.saved_card_repo
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
    let idempotency_key = request.idempotency_key.clone()
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
    state.service_context.payment_repo.create(pending).await?;

    // Charge the card. On error, flip the Pending row to Failed so
    // it doesn't haunt the "Pending older than 5 minutes — investigate"
    // queue (and so the webhook self-heal won't act on a dead row).
    let stripe_payment_id = match stripe_client.charge_saved_card(
        current_user.member.id,
        &card.stripe_payment_method_id,
        amount_cents,
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

    // Race-free flip. If we win (won_flip=true), do the post-work.
    // If the webhook beat us, it already did the post-work and we
    // return success without duplicating dues extension.
    let won_flip = state.service_context.payment_repo
        .complete_pending_payment(payment_id, &stripe_payment_id)
        .await?;
    if !won_flip {
        return Ok((StatusCode::OK, Json(serde_json::json!({
            "payment_id": payment_id,
            "status": "completed",
        }))));
    }

    // Both the dues-extension and the auto-renew branch below run
    // through the shared BillingService.
    let billing_service = state.billing_service.as_ref();

    // Extend dues first so the new dues_paid_until is what auto-renew
    // schedules off of. This is the source of truth for "when does the
    // next renewal fire" — if scheduling reads the old date, the queued
    // charge would fire on the previous cycle's day.
    billing_service
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
            .enable_auto_renew(current_user.member.id, &request.membership_type_slug)
            .await
        {
            tracing::error!(
                "Charged member {} but failed to enable auto-renew: {}",
                current_user.member.id, e,
            );
        } else {
            state.service_context.audit_service.log(
                Some(current_user.member.id),
                "enable_auto_renew",
                "member",
                &current_user.member.id.to_string(),
                None,
                Some(&format!("via saved card payment ({})", request.membership_type_slug)),
                None,
            ).await;
        }
    } else if let Err(e) = billing_service
        .reschedule_after_payment(current_user.member.id, &request.membership_type_slug)
        .await
    {
        tracing::error!(
            "Charged member {} but failed to reschedule auto-renew: {}",
            current_user.member.id, e,
        );
    }

    Ok((StatusCode::OK, Json(serde_json::json!({
        "payment_id": payment_id,
        "status": "completed",
    }))))
}

pub async fn payment_methods_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> impl IntoResponse {
    let base = BaseContext::for_member(&state, &current_user, &session_info).await;

    let stripe_enabled = state.stripe_client.is_some();
    let stripe_publishable_key = state.settings.stripe.publishable_key
        .clone()
        .unwrap_or_default();

    let is_auto_renew = current_user.member.billing_mode
        == crate::domain::BillingMode::CoterieManaged;
    let is_stripe_subscription = current_user.member.billing_mode
        == crate::domain::BillingMode::StripeSubscription;

    let has_default_card = state.service_context.saved_card_repo
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

#[derive(Debug, Deserialize)]
pub struct UpdateAutoRenewRequest {
    pub enable: bool,
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
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    headers: axum::http::HeaderMap,
    Json(request): Json<UpdateAutoRenewRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let ip = crate::api::state::client_ip(&headers, state.settings.server.trust_forwarded_for());
    if !state.money_limiter.check_and_record(ip) {
        return Err(AppError::TooManyRequests);
    }

    let billing_service = state.billing_service.as_ref();

    if request.enable {
        // Need the member's current membership type slug to schedule
        // a renewal — that's where billing period (monthly/yearly) and
        // amount come from.
        let mt = state.service_context.membership_type_service
            .get(current_user.member.membership_type_id).await?
            .ok_or_else(|| AppError::Internal(
                "Member's membership type was deleted; contact an admin.".to_string()
            ))?;

        // Block enabling without a default card UNLESS the member
        // is on a Stripe-managed subscription — in that case the
        // migration will import their cards from Stripe before the
        // first scheduled charge runs, so requiring a Coterie-side
        // card up front would create a chicken-and-egg problem.
        let on_stripe_sub = current_user.member.billing_mode
            == crate::domain::BillingMode::StripeSubscription;
        if !on_stripe_sub {
            let default_card = state.service_context.saved_card_repo
                .find_default_for_member(current_user.member.id).await?;
            if default_card.is_none() {
                return Err(AppError::BadRequest(
                    "Save a default card before enabling auto-renew.".to_string()
                ));
            }
        }

        billing_service
            .enable_auto_renew(current_user.member.id, &mt.slug)
            .await?;

        state.service_context.audit_service.log(
            Some(current_user.member.id),
            "enable_auto_renew",
            "member",
            &current_user.member.id.to_string(),
            None,
            Some("via payment methods page"),
            None,
        ).await;
    } else {
        billing_service
            .disable_auto_renew(current_user.member.id)
            .await?;

        state.service_context.audit_service.log(
            Some(current_user.member.id),
            "disable_auto_renew",
            "member",
            &current_user.member.id.to_string(),
            None,
            Some("via payment methods page"),
            None,
        ).await;
    }

    Ok(Json(serde_json::json!({
        "is_auto_renew": request.enable,
    })))
}

/// HTMX endpoint - list saved cards as HTML
pub async fn saved_cards_html_api(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {
    let cards = state.service_context.saved_card_repo
        .find_by_member(current_user.member.id)
        .await
        .unwrap_or_default();

    let rows = cards.iter()
        .map(crate::web::portal::partials::saved_card_row_from)
        .collect();
    crate::web::portal::partials::saved_card_list(rows)
}

/// HTMX endpoint - delete a saved card
pub async fn delete_card_api(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    axum::extract::Path(card_id): axum::extract::Path<uuid::Uuid>,
) -> Result<StatusCode, AppError> {
    let card = state.service_context.saved_card_repo
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
    let other_cards = state.service_context.saved_card_repo
        .find_by_member(current_user.member.id).await
        .unwrap_or_default()
        .into_iter()
        .filter(|c| c.id != card_id)
        .count();
    let leaving_with_no_cards = other_cards == 0
        && current_user.member.billing_mode == crate::domain::BillingMode::CoterieManaged;

    state.service_context.saved_card_repo.delete(card_id).await?;

    // Detach on Stripe so the card doesn't outlive the Coterie row.
    if let Some(stripe) = state.stripe_client.as_ref() {
        if let Err(e) = stripe.detach_payment_method(&card.stripe_payment_method_id).await {
            tracing::error!(
                "Locally deleted card {} (pm={}) but Stripe detach failed: {}",
                card_id, card.stripe_payment_method_id, e,
            );
        }
    }

    if leaving_with_no_cards {
        state.service_context.integration_manager
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
                    current_user.member.full_name,
                    current_user.member.email,
                ),
            })
            .await;
    }

    Ok(StatusCode::OK)
}

/// HTMX endpoint - set a card as default
pub async fn set_default_card_api(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    axum::extract::Path(card_id): axum::extract::Path<uuid::Uuid>,
) -> Result<StatusCode, AppError> {
    let card = state.service_context.saved_card_repo
        .find_by_id(card_id)
        .await?
        .ok_or(AppError::NotFound("Card not found".to_string()))?;

    if card.member_id != current_user.member.id {
        return Err(AppError::Forbidden);
    }

    state.service_context.saved_card_repo.set_default(current_user.member.id, card_id).await?;
    Ok(StatusCode::OK)
}

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
    pub kind_label: String,    // "Dues" | "Donation" | "Other"
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
    pub kind_label: String,    // "Dues" | "Donation" | "Other"
    pub description: String,
    pub campaign: Option<String>,
    pub payment_method_label: String,  // "Card via Stripe" | "Manual" | "Waived"
    pub generated_on: String,
}

/// Tax-year aggregation page. Lists every Completed payment grouped
/// by calendar year of `paid_at` (falling back to created_at if
/// paid_at is missing — old manual records sometimes lack it). Per
/// year, the page totals dues separately from donations so the donor-
/// or-member can hand the right number to the right place.
pub async fn receipts_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session): Extension<SessionInfo>,
) -> Result<axum::response::Response, AppError> {
    use crate::domain::{PaymentKind, PaymentStatus};
    use std::collections::BTreeMap;

    let payments = state.service_context.payment_repo
        .find_by_member(current_user.member.id)
        .await?;

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

    let mut years: Vec<ReceiptYearDisplay> = by_year.into_iter().map(|(year, items)| {
        let mut dues_cents: i64 = 0;
        let mut donations_cents: i64 = 0;
        let mut lines: Vec<ReceiptLineDisplay> = items.into_iter().map(|p| {
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
            }.to_string();

            let when = p.paid_at.unwrap_or(p.created_at);
            ReceiptLineDisplay {
                payment_id: p.id.to_string(),
                date: when.format("%Y-%m-%d").to_string(),
                description: p.description.clone(),
                kind_label,
                amount_display: format!("${:.2}", p.amount_cents as f64 / 100.0),
            }
        }).collect();

        // Newest-first within the year.
        lines.sort_by(|a, b| b.date.cmp(&a.date));

        ReceiptYearDisplay {
            year,
            dues_total_display: format!("${:.2}", dues_cents as f64 / 100.0),
            donations_total_display: format!("${:.2}", donations_cents as f64 / 100.0),
            items: lines,
        }
    }).collect();

    // Newest year first.
    years.sort_by(|a, b| b.year.cmp(&a.year));

    let template = ReceiptsTemplate {
        base: BaseContext::for_member(&state, &current_user, &session).await,
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
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    axum::extract::Path(payment_id): axum::extract::Path<uuid::Uuid>,
) -> Result<axum::response::Response, AppError> {
    use crate::domain::{PaymentKind, PaymentMethod, PaymentStatus};

    let payment = state.service_context.payment_repo
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

    let settings = &state.service_context.settings_service;
    let raw_org_name = settings.get_value("org.name").await.unwrap_or_default();
    let org_name = if raw_org_name.is_empty() { "Coterie".to_string() } else { raw_org_name };
    let org_address = settings.get_value("org.address").await.unwrap_or_default();
    let org_contact_email = settings.get_value("org.contact_email").await.unwrap_or_default();
    let org_website_url = settings.get_value("org.website_url").await.unwrap_or_default();
    let org_tax_id = settings.get_value("org.tax_id").await.unwrap_or_default();

    let kind_label = match payment.kind {
        PaymentKind::Membership => "Dues",
        PaymentKind::Donation { .. } => "Donation",
        PaymentKind::Other => "Other",
    }.to_string();

    let payment_method_label = match payment.payment_method {
        PaymentMethod::Stripe => "Card via Stripe",
        PaymentMethod::Manual => "Manual",
        PaymentMethod::Waived => "Waived",
    }.to_string();

    let when = payment.paid_at.unwrap_or(payment.created_at);

    // Try to resolve campaign name when applicable.
    let campaign = if let Some(cid) = payment.kind.campaign_id() {
        state.service_context.donation_campaign_repo
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
