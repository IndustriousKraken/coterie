//! Admin roster for a paid event: who is on it, what state the money
//! is in, and the three operator actions the state machine needs a
//! human for — recording an at-the-door payment, comping a seat, and
//! releasing a seat stuck in `PendingPayment` because a webhook never
//! arrived.
//!
//! Money moves through `PaymentService::record_manual`, not through a
//! new payment-writing entry point: at-the-door is a `Manual` event fee
//! and a comp is a `Waived` one, so both audit through the centralized
//! `audit_action` mapping (`manual_event_fee` / `waive_event_fee`).
//!
//! Releasing a stuck seat deliberately issues NO refund — it exists for
//! the never-arrived-webhook case. An operator who needs the money back
//! uses the refund route, which cancels the seat as a side effect.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Extension,
};
use uuid::Uuid;

use crate::{
    api::middleware::auth::CurrentUser,
    domain::{AttendanceStatus, PaymentKind, PaymentMethod, PaymentStatus},
    repository::{EventRepository, PaymentRepository, RosterEntry},
    service::{
        audit_service::AuditService,
        billing_service::BillingService,
        payment_service::{PaymentService, RecordManualPaymentInput},
    },
    web::portal::admin::partials,
};

/// One rendered roster line. Strings, because this is view data — the
/// template shouldn't be re-deriving "is this paid" from two enums.
pub struct RosterRow {
    pub member_id: String,
    pub member_name: String,
    pub member_email: String,
    pub status: String,
    pub payment_state: String,
    pub amount_display: String,
    /// Drives the "release seat" control — only a held-but-unpaid seat
    /// can be released.
    pub is_pending_payment: bool,
}

impl RosterRow {
    pub fn from_entry(e: RosterEntry) -> Self {
        // The absence of a payment is itself the answer: a free RSVP.
        let payment_state = match (&e.payment_status, &e.payment_method) {
            (None, _) => "No charge",
            (Some(PaymentStatus::Pending), _) => "Awaiting payment",
            (Some(PaymentStatus::Failed), _) => "Abandoned",
            (Some(PaymentStatus::Refunded), _) => "Refunded",
            (Some(PaymentStatus::Completed), Some(PaymentMethod::Waived)) => "Comped",
            (Some(PaymentStatus::Completed), Some(PaymentMethod::Manual)) => "Paid at the door",
            (Some(PaymentStatus::Completed), _) => "Paid",
        }
        .to_string();

        Self {
            member_id: e.member_id.to_string(),
            member_name: e.member_name,
            member_email: e.member_email,
            status: format!("{:?}", e.status),
            payment_state,
            amount_display: e
                .amount_cents
                .map(|c| format!("${:.2}", c as f64 / 100.0))
                .unwrap_or_else(|| "—".to_string()),
            is_pending_payment: matches!(e.status, AttendanceStatus::PendingPayment),
        }
    }
}

#[derive(serde::Deserialize)]
pub struct RosterMemberForm {
    pub member_id: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub csrf_token: String,
}

/// Record a cash/at-the-door event fee and seat the member.
pub async fn admin_roster_record_at_the_door(
    State(event_repo): State<Arc<dyn EventRepository>>,
    State(payment_service): State<Arc<PaymentService>>,
    State(billing_service): State<Arc<BillingService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(event_id): Path<String>,
    axum::Form(form): axum::Form<RosterMemberForm>,
) -> impl IntoResponse {
    record_seat_payment(
        event_repo,
        payment_service,
        billing_service,
        current_user,
        &event_id,
        &form.member_id,
        PaymentMethod::Manual,
    )
    .await
}

/// Comp a seat: a `$0` `Waived` event fee, no Stripe charge.
pub async fn admin_roster_comp_seat(
    State(event_repo): State<Arc<dyn EventRepository>>,
    State(payment_service): State<Arc<PaymentService>>,
    State(billing_service): State<Arc<BillingService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(event_id): Path<String>,
    axum::Form(form): axum::Form<RosterMemberForm>,
) -> impl IntoResponse {
    record_seat_payment(
        event_repo,
        payment_service,
        billing_service,
        current_user,
        &event_id,
        &form.member_id,
        PaymentMethod::Waived,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn record_seat_payment(
    event_repo: Arc<dyn EventRepository>,
    payment_service: Arc<PaymentService>,
    billing_service: Arc<BillingService>,
    current_user: CurrentUser,
    event_id: &str,
    member_id: &str,
    method: PaymentMethod,
) -> axum::response::Response {
    let (event_id, member_id) = match (Uuid::parse_str(event_id), Uuid::parse_str(member_id)) {
        (Ok(e), Ok(m)) => (e, m),
        _ => return partials::admin_alert("error", "Invalid ID", false).into_response(),
    };

    let event = match event_repo.find_by_id(event_id).await {
        Ok(Some(e)) => e,
        _ => return partials::admin_alert("error", "Event not found", false).into_response(),
    };

    // A comp is $0 by definition; an at-the-door payment is the event's
    // listed member price.
    let amount_cents = match method {
        PaymentMethod::Waived => 0,
        _ => event.member_price_cents,
    };

    let payment = match payment_service
        .record_manual(
            RecordManualPaymentInput {
                member_id,
                amount_cents,
                kind: PaymentKind::EventFee { event_id },
                description: format!("Event registration — {}", event.title),
                payment_method: method.clone(),
                membership_type_slug: None,
                actor_id: current_user.member.id,
            },
            &billing_service,
        )
        .await
    {
        Ok(p) => p,
        Err(e) => {
            return partials::admin_alert("error", &format!("Error recording: {}", e), false)
                .into_response()
        }
    };

    // Seat them, then point the seat at the payment that bought it.
    // Deliberately bypasses the capacity guard: an admin adding someone
    // at the door has already made that call in the room.
    if let Err(e) = event_repo.register_attendance(event_id, member_id).await {
        return partials::admin_alert(
            "error",
            &format!("Payment recorded but seat not confirmed: {}", e),
            false,
        )
        .into_response();
    }
    if let Err(e) = event_repo
        .link_payment(event_id, member_id, payment.id)
        .await
    {
        tracing::error!(
            "Recorded event-fee payment {} but could not link the seat: {}",
            payment.id,
            e,
        );
    }

    let msg = match method {
        PaymentMethod::Waived => "Seat comped",
        _ => "At-the-door payment recorded",
    };
    partials::admin_alert("success", msg, false).into_response()
}

/// Release a seat stuck in `PendingPayment`. No refund — no money was
/// ever collected for it.
pub async fn admin_roster_release_seat(
    State(event_repo): State<Arc<dyn EventRepository>>,
    State(payment_repo): State<Arc<dyn PaymentRepository>>,
    State(audit_service): State<Arc<AuditService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(event_id): Path<String>,
    axum::Form(form): axum::Form<RosterMemberForm>,
) -> impl IntoResponse {
    let (event_id, member_id) = match (Uuid::parse_str(&event_id), Uuid::parse_str(&form.member_id))
    {
        (Ok(e), Ok(m)) => (e, m),
        _ => return partials::admin_alert("error", "Invalid ID", false).into_response(),
    };

    // Fail the placeholder first so the abandoned checkout can never
    // settle into a Completed payment for a seat that no longer exists.
    if let Some(payment) = payment_repo
        .find_event_fee_payment(event_id, member_id)
        .await
        .ok()
        .flatten()
    {
        if payment.status == PaymentStatus::Pending {
            if let Err(e) = payment_repo.fail_pending_payment(payment.id).await {
                return partials::admin_alert(
                    "error",
                    &format!("Could not release the pending payment: {}", e),
                    false,
                )
                .into_response();
            }
        }
    }

    if let Err(e) = event_repo.release_seat(event_id, member_id).await {
        return partials::admin_alert("error", &format!("Error releasing seat: {}", e), false)
            .into_response();
    }

    audit_service
        .log(
            Some(current_user.member.id),
            "release_event_seat",
            "event",
            &event_id.to_string(),
            None,
            Some(&format!("member {}", member_id)),
            None,
        )
        .await;

    partials::admin_alert("success", "Seat released", false).into_response()
}
