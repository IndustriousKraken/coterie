//! Admin controls for a paid class: what it costs, and who bought it.
//!
//! The series-scope counterpart of `roster.rs`, and deliberately the same
//! three operator actions against the same money path — an at-the-door
//! pass is a `Manual` `SeriesPass` payment and a comp is a `Waived` one,
//! so both go through `PaymentService::record_manual` and audit through
//! the centralized mapping (`manual_series_pass` / `waive_series_pass`).
//! Refunding is the existing `/portal/admin/payments/:id/refund` route,
//! which cancels the enrollment and its future sessions as a side effect.
//!
//! Releasing a stuck enrollment issues NO refund — it exists for the
//! never-arrived-webhook case — and for that reason REFUSES to touch an
//! enrollment whose pass is already `Completed`: no-refund is only safe
//! while no money moved.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Extension,
};
use uuid::Uuid;

use crate::{
    api::middleware::auth::CurrentUser,
    domain::{Attendee, PaymentKind, PaymentMethod, PaymentStatus},
    repository::{
        EventRepository, EventSeriesRepository, PaymentRepository, SeriesEnrollmentRepository,
    },
    service::{
        audit_service::AuditService,
        billing_service::BillingService,
        payment_service::{PaymentService, RecordManualPaymentInput},
        series_enrollment_service::{class_title, seat_future_occurrences},
    },
    web::portal::admin::partials,
};

use super::RosterMemberForm;

/// Pass prices + class capacity + the guest-enrollment toggle, posted
/// from the series detail page.
#[derive(serde::Deserialize)]
pub struct SeriesPricingForm {
    #[serde(default)]
    pub member_price: String,
    #[serde(default)]
    pub guest_price: String,
    #[serde(default)]
    pub capacity: String,
    /// An unchecked checkbox sends no field at all. A bare `String`
    /// parses the `=on` a checked one sends; `Option<bool>` would 400.
    pub guest_registration_enabled: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub csrf_token: String,
}

/// POST — set the class's pass pricing.
///
/// The bounded-class rule lives in `validate_pass_pricing`, so an
/// operator pricing an open-ended series gets a message naming the
/// missing end date rather than an unsellable class.
pub async fn admin_update_series_pricing(
    State(series_repo): State<Arc<dyn EventSeriesRepository>>,
    State(audit_service): State<Arc<AuditService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(series_id): Path<String>,
    axum::Form(form): axum::Form<SeriesPricingForm>,
) -> impl IntoResponse {
    let Ok(series_id) = Uuid::parse_str(&series_id) else {
        return partials::admin_alert("error", "Invalid series ID", false).into_response();
    };
    let series = match series_repo.find_by_id(series_id).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return partials::admin_alert("error", "Series not found", false).into_response()
        }
        Err(e) => {
            return partials::admin_alert("error", &format!("Error loading series: {}", e), false)
                .into_response()
        }
    };

    let pricing = crate::domain::SeriesPassPricing {
        member_price_cents: match super::single::parse_price(&form.member_price) {
            Ok(c) => c,
            Err(msg) => return partials::admin_alert("error", msg, false).into_response(),
        },
        guest_price_cents: match super::single::parse_price(&form.guest_price) {
            Ok(c) => c,
            Err(msg) => return partials::admin_alert("error", msg, false).into_response(),
        },
        guest_registration_enabled: form.guest_registration_enabled.is_some(),
        // A cleared or non-positive capacity means "no limit", which is
        // what an operator emptying the field intends.
        max_enrollments: form.capacity.trim().parse::<i32>().ok().filter(|c| *c > 0),
    };

    if let Err(msg) = crate::domain::validate_pass_pricing(&pricing, series.until_date) {
        return partials::admin_alert("error", &msg, false).into_response();
    }

    if let Err(e) = series_repo.set_pricing(series_id, &pricing).await {
        return partials::admin_alert("error", &format!("Error saving pricing: {}", e), false)
            .into_response();
    }

    audit_service
        .log(
            Some(current_user.member.id),
            "update_series_pricing",
            "event_series",
            &series_id.to_string(),
            None,
            Some(&format!(
                "member ${:.2} / guest ${:.2}, capacity {}",
                pricing.member_price_cents as f64 / 100.0,
                pricing.guest_price_cents as f64 / 100.0,
                pricing
                    .max_enrollments
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "unlimited".to_string()),
            )),
            None,
        )
        .await;

    partials::admin_alert("success", "Class pricing saved", false).into_response()
}

/// Record a cash/at-the-door class pass and enroll the buyer.
#[allow(clippy::too_many_arguments)]
pub async fn admin_enrollment_record_at_the_door(
    State(series_repo): State<Arc<dyn EventSeriesRepository>>,
    State(event_repo): State<Arc<dyn EventRepository>>,
    State(enrollment_repo): State<Arc<dyn SeriesEnrollmentRepository>>,
    State(payment_service): State<Arc<PaymentService>>,
    State(billing_service): State<Arc<BillingService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(series_id): Path<String>,
    axum::Form(form): axum::Form<RosterMemberForm>,
) -> impl IntoResponse {
    record_enrollment_payment(
        series_repo,
        event_repo,
        enrollment_repo,
        payment_service,
        billing_service,
        current_user,
        &series_id,
        &form,
        PaymentMethod::Manual,
    )
    .await
}

/// Comp an enrollment: a `$0` `Waived` class pass, no Stripe charge.
#[allow(clippy::too_many_arguments)]
pub async fn admin_enrollment_comp(
    State(series_repo): State<Arc<dyn EventSeriesRepository>>,
    State(event_repo): State<Arc<dyn EventRepository>>,
    State(enrollment_repo): State<Arc<dyn SeriesEnrollmentRepository>>,
    State(payment_service): State<Arc<PaymentService>>,
    State(billing_service): State<Arc<BillingService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(series_id): Path<String>,
    axum::Form(form): axum::Form<RosterMemberForm>,
) -> impl IntoResponse {
    record_enrollment_payment(
        series_repo,
        event_repo,
        enrollment_repo,
        payment_service,
        billing_service,
        current_user,
        &series_id,
        &form,
        PaymentMethod::Waived,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn record_enrollment_payment(
    series_repo: Arc<dyn EventSeriesRepository>,
    event_repo: Arc<dyn EventRepository>,
    enrollment_repo: Arc<dyn SeriesEnrollmentRepository>,
    payment_service: Arc<PaymentService>,
    billing_service: Arc<BillingService>,
    current_user: CurrentUser,
    series_id: &str,
    form: &RosterMemberForm,
    method: PaymentMethod,
) -> axum::response::Response {
    let (series_id, enrollee) = match (Uuid::parse_str(series_id), form.attendee()) {
        (Ok(s), Some(a)) => (s, a),
        _ => return partials::admin_alert("error", "Invalid ID", false).into_response(),
    };

    let series = match series_repo.find_by_id(series_id).await {
        Ok(Some(s)) => s,
        _ => return partials::admin_alert("error", "Series not found", false).into_response(),
    };
    let class_title = class_title(&*event_repo, series_id).await;

    // A comp is $0 by definition; an at-the-door pass is the price that
    // applies to who is paying it.
    let amount_cents = match (&method, &enrollee) {
        (PaymentMethod::Waived, _) => 0,
        (_, Attendee::Member(_)) => series.member_price_cents,
        (_, Attendee::Guest { .. }) => series.guest_price_cents,
    };

    let payment = match payment_service
        .record_manual(
            RecordManualPaymentInput {
                payer: enrollee.as_payer(),
                amount_cents,
                kind: PaymentKind::SeriesPass { series_id },
                description: format!("Class pass — {}", class_title),
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

    // Enroll them, point the enrollment at the payment that bought it,
    // then materialize attendance for every session still to come.
    // Deliberately bypasses the capacity guard: an admin adding someone at
    // the door has already made that call in the room.
    if let Err(e) = enrollment_repo.register(series_id, &enrollee).await {
        return partials::admin_alert(
            "error",
            &format!("Payment recorded but enrollment not confirmed: {}", e),
            false,
        )
        .into_response();
    }
    if let Err(e) = enrollment_repo
        .link_payment(series_id, &enrollee, payment.id)
        .await
    {
        tracing::error!(
            "Recorded series-pass payment {} but could not link the enrollment: {}",
            payment.id,
            e,
        );
    }
    if let Err(e) =
        seat_future_occurrences(&*event_repo, series_id, &enrollee, Some(payment.id)).await
    {
        return partials::admin_alert(
            "error",
            &format!("Enrolled, but attendance could not be created: {}", e),
            false,
        )
        .into_response();
    }

    let msg = match method {
        PaymentMethod::Waived => "Enrollment comped",
        _ => "At-the-door class payment recorded",
    };
    partials::admin_alert("success", msg, false).into_response()
}

/// Release an enrollment stuck in `PendingPayment`. No refund — no money
/// was ever collected for it.
pub async fn admin_enrollment_release(
    State(enrollment_repo): State<Arc<dyn SeriesEnrollmentRepository>>,
    State(payment_repo): State<Arc<dyn PaymentRepository>>,
    State(audit_service): State<Arc<AuditService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(series_id): Path<String>,
    axum::Form(form): axum::Form<RosterMemberForm>,
) -> impl IntoResponse {
    let (series_id, enrollee) = match (Uuid::parse_str(&series_id), form.attendee()) {
        (Ok(s), Some(a)) => (s, a),
        _ => return partials::admin_alert("error", "Invalid ID", false).into_response(),
    };

    if let Some(payment) = payment_repo
        .find_series_pass_payment(series_id, &enrollee.as_payer())
        .await
        .ok()
        .flatten()
    {
        match payment.status {
            // Fail the placeholder first so the abandoned checkout can
            // never settle into a Completed payment for an enrollment
            // that no longer exists.
            PaymentStatus::Pending => {
                if let Err(e) = payment_repo.fail_pending_payment(payment.id).await {
                    return partials::admin_alert(
                        "error",
                        &format!("Could not release the pending payment: {}", e),
                        false,
                    )
                    .into_response();
                }
            }
            // Money WAS taken, even though the enrollment is still
            // PendingPayment. Releasing here would delete a paid place
            // with no refund, which is the one thing this control must
            // never do. Refunding cancels the enrollment as a side effect.
            PaymentStatus::Completed if payment.payment_method != PaymentMethod::Waived => {
                return partials::admin_alert(
                    "error",
                    "This enrollee has already paid — refund the payment to release their \
                     place. Releasing issues no refund.",
                    false,
                )
                .into_response();
            }
            _ => {}
        }
    }

    if let Err(e) = enrollment_repo.release(series_id, &enrollee).await {
        return partials::admin_alert(
            "error",
            &format!("Error releasing enrollment: {}", e),
            false,
        )
        .into_response();
    }

    audit_service
        .log(
            Some(current_user.member.id),
            "release_series_enrollment",
            "event_series",
            &series_id.to_string(),
            None,
            Some(&match &enrollee {
                Attendee::Member(id) => format!("member {}", id),
                Attendee::Guest { email, .. } => format!("guest {}", email),
            }),
            None,
        )
        .await;

    partials::admin_alert("success", "Enrollment released", false).into_response()
}
