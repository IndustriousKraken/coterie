use axum::{
    extract::{Path, State, Query},
    http::{HeaderMap, StatusCode},
    Json,
    Extension,
    response::IntoResponse,
};
use chrono::{Months, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    api::{state::AppState, middleware::auth::CurrentUser},
    domain::{Payment, PaymentMethod, PaymentStatus, SavedCard, configurable_types::BillingPeriod},
    error::{AppError, Result},
};

#[derive(Debug, Deserialize)]
pub struct CreatePaymentRequest {
    /// The slug of the membership type (e.g., "regular", "student", "corporate").
    /// The amount charged is always the server-side fee on the matching
    /// membership_types row — never a client-supplied value.
    pub membership_type_slug: String,
}

#[derive(Debug, Serialize)]
pub struct CreatePaymentResponse {
    pub payment_id: Uuid,
    pub checkout_url: String,
}

#[derive(Debug, Deserialize)]
pub struct ListPaymentsQuery {
    pub status: Option<PaymentStatus>,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ManualPaymentRequest {
    pub member_id: Uuid,
    pub amount_cents: i64,
    pub membership_type_slug: Option<String>,
    pub description: String,
}

#[derive(Debug, Deserialize)]
pub struct WaivePaymentRequest {
    pub member_id: Uuid,
    pub membership_type_slug: Option<String>,
    pub description: String,
}

fn is_admin(user: &CurrentUser) -> bool {
    user.member.is_admin
}

pub async fn create(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Json(request): Json<CreatePaymentRequest>,
) -> Result<(StatusCode, Json<CreatePaymentResponse>)> {
    // Check if Stripe is configured
    if !state.stripe_client.is_some() {
        return Err(AppError::ServiceUnavailable("Payment processing is not configured".to_string()));
    }

    let stripe_client = state.stripe_client.as_ref().unwrap();

    // Look up membership type from database to get pricing
    let membership_type = state.service_context.membership_type_service
        .get_by_slug(&request.membership_type_slug)
        .await?
        .ok_or_else(|| AppError::NotFound(format!(
            "Membership type '{}' not found",
            request.membership_type_slug
        )))?;

    // Check if the membership type is active
    if !membership_type.is_active {
        return Err(AppError::BadRequest(format!(
            "Membership type '{}' is not currently available",
            membership_type.name
        )));
    }

    let amount_cents = membership_type.fee_cents as i64;

    // Create checkout session
    let (checkout_url, payment_id) = stripe_client.create_membership_checkout_session(
        user.member.id,
        &membership_type.name,
        &membership_type.slug,
        amount_cents,
        format!("{}/portal/payments/success", state.settings.server.base_url),
        format!("{}/portal/payments/cancel", state.settings.server.base_url),
    ).await?;

    let response = CreatePaymentResponse {
        payment_id,
        checkout_url,
    };

    Ok((StatusCode::CREATED, Json(response)))
}

pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<Payment>> {
    let payment = state.service_context.payment_repo
        .find_by_id(id)
        .await?
        .ok_or(AppError::NotFound("Payment not found".to_string()))?;

    // Check if user can view this payment (must be the payer or admin)
    if payment.member_id != user.member.id && !is_admin(&user) {
        return Err(AppError::Forbidden);
    }

    Ok(Json(payment))
}

pub async fn list_by_member(
    State(state): State<AppState>,
    Path(member_id): Path<Uuid>,
    Query(params): Query<ListPaymentsQuery>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<Vec<Payment>>> {
    // Check if user can view these payments (must be the member or admin)
    if member_id != user.member.id && !is_admin(&user) {
        return Err(AppError::Forbidden);
    }

    let payments = state.service_context.payment_repo
        .find_by_member(member_id)
        .await?;

    // Filter by status if requested
    let filtered: Vec<Payment> = if let Some(status) = params.status {
        payments.into_iter()
            .filter(|p| p.status == status)
            .collect()
    } else {
        payments
    };

    // Apply limit
    let limited: Vec<Payment> = filtered.into_iter()
        .take(params.limit.unwrap_or(50) as usize)
        .collect();

    Ok(Json(limited))
}

pub async fn stripe_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<impl IntoResponse> {
    // Check if Stripe is configured
    if !state.stripe_client.is_some() {
        return Ok(StatusCode::SERVICE_UNAVAILABLE);
    }

    let stripe_client = state.stripe_client.as_ref().unwrap();

    // Get Stripe signature from headers
    let stripe_signature = headers
        .get("stripe-signature")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::BadRequest("Missing Stripe signature".to_string()))?;

    // Handle the webhook
    stripe_client.handle_webhook(&body, stripe_signature).await?;

    Ok(StatusCode::OK)
}

/// Extend a member's dues_paid_until based on a membership type's billing period.
async fn extend_member_dues(
    state: &AppState,
    member_id: Uuid,
    membership_type_slug: &str,
) -> Result<()> {
    let membership_type = state.service_context.membership_type_service
        .get_by_slug(membership_type_slug)
        .await?
        .ok_or_else(|| AppError::NotFound(format!(
            "Membership type '{}' not found", membership_type_slug
        )))?;

    let billing_period = membership_type.billing_period_enum()
        .unwrap_or(BillingPeriod::Yearly);

    let row = sqlx::query_scalar::<_, Option<chrono::DateTime<Utc>>>(
        "SELECT dues_paid_until FROM members WHERE id = ?"
    )
        .bind(member_id.to_string())
        .fetch_optional(&state.service_context.db_pool)
        .await
        .map_err(|e| AppError::Internal(format!("Database error: {}", e)))?;

    let current_dues = row.flatten();
    let now = Utc::now();
    let base_date = current_dues
        .filter(|d| *d > now)
        .unwrap_or(now);

    let new_dues_date = match billing_period {
        BillingPeriod::Monthly => base_date.checked_add_months(Months::new(1)).unwrap_or(base_date),
        BillingPeriod::Yearly => base_date.checked_add_months(Months::new(12)).unwrap_or(base_date),
        BillingPeriod::Lifetime => chrono::DateTime::<Utc>::MAX_UTC,
    };

    sqlx::query("UPDATE members SET dues_paid_until = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(new_dues_date)
        .bind(member_id.to_string())
        .execute(&state.service_context.db_pool)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to update dues: {}", e)))?;

    tracing::info!(
        "Extended dues for member {} to {} (billing period: {:?})",
        member_id, new_dues_date, billing_period
    );

    Ok(())
}

pub async fn create_manual(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Json(request): Json<ManualPaymentRequest>,
) -> Result<(StatusCode, Json<Payment>)> {
    // Admin auth is enforced by the admin_routes middleware

    let payment = Payment {
        id: Uuid::new_v4(),
        member_id: request.member_id,
        amount_cents: request.amount_cents,
        currency: "USD".to_string(),
        status: PaymentStatus::Completed,
        payment_method: PaymentMethod::Manual,
        stripe_payment_id: None,
        description: request.description.clone(),
        paid_at: Some(Utc::now()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let payment = state.service_context.payment_repo.create(payment).await?;

    // Extend dues if membership_type_slug provided
    if let Some(slug) = &request.membership_type_slug {
        extend_member_dues(&state, request.member_id, slug).await?;
    }

    state.service_context.audit_service.log(
        Some(user.member.id),
        "manual_payment",
        "member",
        &request.member_id.to_string(),
        None,
        Some(&format!("${:.2} — {}", request.amount_cents as f64 / 100.0, request.description)),
        None,
    ).await;

    Ok((StatusCode::CREATED, Json(payment)))
}

pub async fn waive(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Json(request): Json<WaivePaymentRequest>,
) -> Result<(StatusCode, Json<Payment>)> {
    // Admin auth is enforced by the admin_routes middleware

    let payment = Payment {
        id: Uuid::new_v4(),
        member_id: request.member_id,
        amount_cents: 0,
        currency: "USD".to_string(),
        status: PaymentStatus::Completed,
        payment_method: PaymentMethod::Waived,
        stripe_payment_id: None,
        description: request.description.clone(),
        paid_at: Some(Utc::now()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let payment = state.service_context.payment_repo.create(payment).await?;

    // Extend dues if membership_type_slug provided
    if let Some(slug) = &request.membership_type_slug {
        extend_member_dues(&state, request.member_id, slug).await?;
    }

    state.service_context.audit_service.log(
        Some(user.member.id),
        "waive_dues",
        "member",
        &request.member_id.to_string(),
        None,
        Some(&request.description),
        None,
    ).await;

    Ok((StatusCode::CREATED, Json(payment)))
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
