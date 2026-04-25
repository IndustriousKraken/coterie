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
    web::templates::{HtmlTemplate, UserInfo},
};
use super::is_admin;

#[derive(Template)]
#[template(path = "portal/payments.html")]
pub struct PaymentsTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
}

#[derive(Template)]
#[template(path = "portal/payment_new.html")]
pub struct PaymentNewTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub stripe_enabled: bool,
    pub membership_types: Vec<MembershipTypeDisplay>,
    pub saved_cards: Vec<SavedCardDisplay>,
    pub stripe_publishable_key: String,
    pub csrf_token: String,
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
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
}

#[derive(Template)]
#[template(path = "portal/payment_cancel.html")]
pub struct PaymentCancelTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
}

#[derive(Template)]
#[template(path = "portal/payment_methods.html")]
pub struct PaymentMethodsTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub stripe_enabled: bool,
    pub stripe_publishable_key: String,
    pub csrf_token: String,
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
    State(_state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {
    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let template = PaymentsTemplate {
        current_user: Some(user_info),
        is_admin: is_admin(&current_user.member),
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

    if payments.is_empty() {
        return axum::response::Html(
            r#"<div class="p-6 text-center text-gray-500">
                No payment history
            </div>"#.to_string()
        );
    }

    let mut html = String::from(r#"<div class="divide-y">"#);

    for payment in payments {
        let status_badge = match format!("{:?}", payment.status).as_str() {
            "Completed" => r#"<span class="px-2 py-1 text-xs font-medium rounded bg-green-100 text-green-800">Completed</span>"#,
            "Pending" => r#"<span class="px-2 py-1 text-xs font-medium rounded bg-yellow-100 text-yellow-800">Pending</span>"#,
            "Failed" => r#"<span class="px-2 py-1 text-xs font-medium rounded bg-red-100 text-red-800">Failed</span>"#,
            "Refunded" => r#"<span class="px-2 py-1 text-xs font-medium rounded bg-gray-100 text-gray-800">Refunded</span>"#,
            _ => "",
        };

        let description = if payment.description.is_empty() {
            "Membership dues".to_string()
        } else {
            crate::web::escape_html(&payment.description)
        };

        html.push_str(&format!(
            r#"<div class="px-6 py-4 flex justify-between items-center">
                <div>
                    <p class="font-medium text-gray-900">{}</p>
                    <p class="text-sm text-gray-500">{}</p>
                </div>
                <div class="text-right">
                    <p class="font-medium text-gray-900">${:.2}</p>
                    <div class="mt-1">{}</div>
                </div>
            </div>"#,
            description,
            payment.created_at.format("%B %d, %Y"),
            payment.amount_cents as f64 / 100.0,
            status_badge
        ));
    }

    html.push_str("</div>");
    axum::response::Html(html)
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
    let status = if let Some(dues_until) = current_user.member.dues_paid_until {
        if dues_until > chrono::Utc::now() {
            r#"<span class="text-green-600">Current</span>"#
        } else {
            r#"<span class="text-red-600">Expired</span>"#
        }
    } else {
        r#"<span class="text-yellow-600">Unpaid</span>"#
    };

    axum::response::Html(status.to_string())
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
    State(_state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {
    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let template = PaymentSuccessTemplate {
        current_user: Some(user_info),
        is_admin: is_admin(&current_user.member),
    };

    HtmlTemplate(template)
}

pub async fn payment_cancel_page(
    State(_state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {
    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let template = PaymentCancelTemplate {
        current_user: Some(user_info),
        is_admin: is_admin(&current_user.member),
    };

    HtmlTemplate(template)
}

pub async fn payment_new_page(
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
        current_user: Some(user_info),
        is_admin: is_admin(&current_user.member),
        stripe_enabled,
        membership_types,
        saved_cards,
        stripe_publishable_key,
        csrf_token,
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
    Json(request): Json<ChargeSavedCardRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), AppError> {
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

    // Charge the card
    let stripe_payment_id = stripe_client.charge_saved_card(
        current_user.member.id,
        &card.stripe_payment_method_id,
        amount_cents,
        &description,
        &idempotency_key,
    ).await?;

    // Create payment record
    let payment = crate::domain::Payment {
        id: uuid::Uuid::new_v4(),
        member_id: current_user.member.id,
        amount_cents,
        currency: "USD".to_string(),
        status: crate::domain::PaymentStatus::Completed,
        payment_method: crate::domain::PaymentMethod::Stripe,
        stripe_payment_id: Some(stripe_payment_id),
        description,
        paid_at: Some(chrono::Utc::now()),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    let payment = state.service_context.payment_repo.create(payment).await?;

    // Extend dues first so the new dues_paid_until is what auto-renew
    // schedules off of. This is the source of truth for "when does the
    // next renewal fire" — if scheduling reads the old date, the queued
    // charge would fire on the previous cycle's day.
    stripe_client.extend_member_dues(current_user.member.id, &membership_type.slug).await?;

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
    let billing_service = state.service_context.billing_service(
        state.stripe_client.clone(),
        state.settings.server.base_url.clone(),
    );
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
        "payment_id": payment.id,
        "status": "completed",
    }))))
}

pub async fn payment_methods_page(
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

    let template = PaymentMethodsTemplate {
        current_user: Some(user_info),
        is_admin: is_admin(&current_user.member),
        stripe_enabled,
        stripe_publishable_key,
        csrf_token,
    };

    HtmlTemplate(template)
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

    if cards.is_empty() {
        return axum::response::Html(
            r#"<div class="p-6 text-center text-gray-500">
                No saved payment methods. Add a card below.
            </div>"#.to_string()
        );
    }

    let mut html = String::from(r#"<div class="divide-y">"#);

    for card in cards {
        let default_badge = if card.is_default {
            r#"<span class="ml-2 px-2 py-1 text-xs font-medium rounded bg-blue-100 text-blue-800">Default</span>"#
        } else {
            ""
        };

        let expired_badge = if card.is_expired() {
            r#"<span class="ml-2 px-2 py-1 text-xs font-medium rounded bg-red-100 text-red-800">Expired</span>"#
        } else {
            ""
        };

        let set_default_btn = if !card.is_default {
            format!(
                r#"<button
                    hx-put="/portal/api/payments/cards/{}/default"
                    hx-swap="none"
                    hx-on::after-request="htmx.trigger('#saved-cards', 'refresh')"
                    class="text-blue-600 hover:text-blue-800 text-sm mr-3">
                    Set Default
                </button>"#,
                card.id
            )
        } else {
            String::new()
        };

        html.push_str(&format!(
            r#"<div class="px-6 py-4 flex justify-between items-center">
                <div class="flex items-center">
                    <span class="font-medium text-gray-900">{}</span>
                    <span class="ml-4 text-sm text-gray-500">Exp: {}</span>
                    {}{}
                </div>
                <div>
                    {}
                    <button
                        hx-delete="/portal/api/payments/cards/{}"
                        hx-confirm="Are you sure you want to remove this card?"
                        hx-swap="none"
                        hx-on::after-request="htmx.trigger('#saved-cards', 'refresh')"
                        class="text-red-600 hover:text-red-800 text-sm">
                        Remove
                    </button>
                </div>
            </div>"#,
            card.display_name(),
            card.exp_display(),
            default_badge,
            expired_badge,
            set_default_btn,
            card.id
        ));
    }

    html.push_str("</div>");
    axum::response::Html(html)
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

    state.service_context.saved_card_repo.delete(card_id).await?;
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
