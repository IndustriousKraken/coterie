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
    domain::{Payment, PaymentMethod, PaymentStatus, configurable_types::BillingPeriod},
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
    user.member.notes
        .as_ref()
        .map(|n| n.contains("ADMIN"))
        .unwrap_or(false)
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
    Extension(_user): Extension<CurrentUser>,
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
        description: request.description,
        paid_at: Some(Utc::now()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let payment = state.service_context.payment_repo.create(payment).await?;

    // Extend dues if membership_type_slug provided
    if let Some(slug) = &request.membership_type_slug {
        extend_member_dues(&state, request.member_id, slug).await?;
    }

    Ok((StatusCode::CREATED, Json(payment)))
}

pub async fn waive(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
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
        description: request.description,
        paid_at: Some(Utc::now()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let payment = state.service_context.payment_repo.create(payment).await?;

    // Extend dues if membership_type_slug provided
    if let Some(slug) = &request.membership_type_slug {
        extend_member_dues(&state, request.member_id, slug).await?;
    }

    Ok((StatusCode::CREATED, Json(payment)))
}
