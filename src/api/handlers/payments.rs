use axum::{
    extract::{Path, State, Query},
    http::{HeaderMap, StatusCode},
    Json,
    Extension,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    api::{state::AppState, middleware::auth::CurrentUser},
    domain::{Payment, PaymentStatus},
    error::{AppError, Result},
};

#[derive(Debug, Deserialize)]
pub struct CreatePaymentRequest {
    /// The slug of the membership type (e.g., "regular", "student", "corporate")
    pub membership_type_slug: String,
    /// Optional override amount in cents. If not provided, uses the fee from membership_types table
    pub amount_cents: Option<i64>,
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
    pub description: String,
}

#[derive(Debug, Deserialize)]
pub struct WaivePaymentRequest {
    pub member_id: Uuid,
    pub description: String,
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

    // Use the fee from the database, or allow override if provided
    let amount_cents = request.amount_cents.unwrap_or(membership_type.fee_cents as i64);

    // Create checkout session
    let checkout_url = stripe_client.create_membership_checkout_session(
        user.member.id,
        &membership_type.name,
        amount_cents,
        format!("{}/payment/success", state.settings.server.base_url),
        format!("{}/payment/cancel", state.settings.server.base_url),
    ).await?;

    let response = CreatePaymentResponse {
        payment_id: Uuid::new_v4(), // This will be replaced with actual payment ID
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
    if payment.member_id != user.member.id {
        // TODO: Add admin check
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
    if member_id != user.member.id {
        // TODO: Add admin check
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

pub async fn create_manual(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
    Json(request): Json<ManualPaymentRequest>,
) -> Result<(StatusCode, Json<Payment>)> {
    // TODO: Check if user is admin
    
    if !state.stripe_client.is_some() {
        return Err(AppError::ServiceUnavailable("Payment processing is not configured".to_string()));
    }

    let stripe_client = state.stripe_client.as_ref().unwrap();
    
    let payment = stripe_client.create_manual_payment(
        request.member_id,
        request.amount_cents,
        request.description,
    ).await?;
    
    Ok((StatusCode::CREATED, Json(payment)))
}

pub async fn waive(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
    Json(request): Json<WaivePaymentRequest>,
) -> Result<(StatusCode, Json<Payment>)> {
    // TODO: Check if user is admin
    
    if !state.stripe_client.is_some() {
        return Err(AppError::ServiceUnavailable("Payment processing is not configured".to_string()));
    }

    let stripe_client = state.stripe_client.as_ref().unwrap();
    
    let payment = stripe_client.waive_payment(
        request.member_id,
        request.description,
    ).await?;
    
    Ok((StatusCode::CREATED, Json(payment)))
}