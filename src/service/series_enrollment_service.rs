//! Enrollment in a paid class — one payment buys a place in every
//! remaining session of a bounded recurring series.
//!
//! This is a41's seat machinery at series scope, not a second state
//! machine. The ordering is identical and for the identical reason:
//!
//!   - the place is claimed BEFORE the Checkout session is created, so a
//!     full class can never mint a session and turn a rejected click into
//!     a refund incident;
//!   - if the session can't be created, the claim is released;
//!   - the enrollment is confirmed by the webhook, never by the browser's
//!     return to `success_url`.
//!
//! What a confirmed enrollment BUYS is `event_attendance` rows — one per
//! occurrence that has not yet started. That is the whole design: rosters,
//! check-in, reminders, and iCal keep reading one table, and "who is
//! coming on week three" has exactly one answer. Occurrences that already
//! started are never back-filled, because a roster asserting attendance at
//! a session that already happened is a lie check-in data would inherit.
//!
//! The free functions at the bottom exist because the completion webhook
//! and the refund path need this logic too, and the dispatcher is built by
//! `StripeHandle` before any service exists. They take repositories, so
//! there is one implementation of "confirm and seat" rather than two.

use std::sync::Arc;

use uuid::Uuid;

use crate::{
    domain::{
        AttendanceStatus, Attendee, EventSeries, Member, MemberStatus, PaymentStatus, StripeRef,
        MAX_PAYMENT_CENTS,
    },
    error::{AppError, Result},
    payments::StripeHandle,
    repository::{EventRepository, PaymentRepository, SeriesEnrollmentRepository},
    service::event_registration_service::{guest_attendee, RegistrationOutcome},
};

pub struct SeriesEnrollmentService {
    event_repo: Arc<dyn EventRepository>,
    enrollment_repo: Arc<dyn SeriesEnrollmentRepository>,
    payment_repo: Arc<dyn PaymentRepository>,
    /// Read `current()` per call so a portal key rotation reaches this
    /// path without a restart, same as every other money path.
    stripe_handle: Arc<StripeHandle>,
    base_url: String,
}

impl SeriesEnrollmentService {
    pub fn new(
        event_repo: Arc<dyn EventRepository>,
        enrollment_repo: Arc<dyn SeriesEnrollmentRepository>,
        payment_repo: Arc<dyn PaymentRepository>,
        stripe_handle: Arc<StripeHandle>,
        base_url: String,
    ) -> Self {
        Self {
            event_repo,
            enrollment_repo,
            payment_repo,
            stripe_handle,
            base_url,
        }
    }

    /// Enroll `member` in `series`, charging when the class has a member
    /// pass price. `class_title` names the class on the Checkout line item
    /// and the receipt — it comes from an occurrence, since a series row
    /// carries no title of its own.
    pub async fn enroll(
        &self,
        member: &Member,
        series: &EventSeries,
        class_title: &str,
    ) -> Result<RegistrationOutcome> {
        // Mirrors the single-event rule: only Active/Honorary members
        // register. Checked here as well as in middleware so a future
        // non-portal caller can't claim a place for an expired member.
        if !matches!(member.status, MemberStatus::Active | MemberStatus::Honorary) {
            return Err(AppError::BadRequest(
                "Only active members can enroll in a class".to_string(),
            ));
        }

        let enrollee = Attendee::Member(member.id);
        if !series.is_paid_class() {
            return self.enroll_free(series, &enrollee).await;
        }
        self.hold_and_charge(series, &enrollee, series.member_price_cents, class_title)
            .await
    }

    /// Enroll a guest (non-member) at the guest pass price.
    ///
    /// The supplied email is NEVER looked up against the member
    /// directory, for the reason a42 records: matching it would let anyone
    /// write an enrollment and a payment into a named member's account by
    /// typing their address. Callers own the protections in front of this
    /// (rate limit, then bot challenge) and the `publicly_enrollable`
    /// check — this method is the money, not the door.
    pub async fn enroll_guest(
        &self,
        series: &EventSeries,
        class_title: &str,
        name: &str,
        email: &str,
    ) -> Result<RegistrationOutcome> {
        let enrollee = guest_attendee(name, email)?;
        if !series.is_paid_for_guests() {
            return self.enroll_free(series, &enrollee).await;
        }
        self.hold_and_charge(series, &enrollee, series.guest_price_cents, class_title)
            .await
    }

    /// Free class: claim through the capacity guard, then confirm. No
    /// payment row and no Checkout session — the same short-circuit a41's
    /// free path takes, with the capacity check kept because a free door
    /// is exactly where place-squatting shows up.
    async fn enroll_free(
        &self,
        series: &EventSeries,
        enrollee: &Attendee,
    ) -> Result<RegistrationOutcome> {
        // Re-submitting returns the existing enrollment rather than
        // resetting it — claiming again would drop a confirmed enrollment
        // back to `PendingPayment`.
        if let Some(existing) = self.enrollment_repo.find(series.id, enrollee).await? {
            if existing.status == AttendanceStatus::Registered {
                return Ok(RegistrationOutcome::Registered);
            }
        }

        self.enrollment_repo
            .claim(series.id, enrollee, series.max_enrollments)
            .await?;
        self.enrollment_repo.register(series.id, enrollee).await?;
        seat_future_occurrences(&*self.event_repo, series.id, enrollee, None).await?;
        Ok(RegistrationOutcome::Registered)
    }

    /// The paid path: double-charge guard → claim the place → mint the
    /// Checkout session → link the payment, releasing the claim if the
    /// session can't be created. Same ordering as a single paid seat.
    async fn hold_and_charge(
        &self,
        series: &EventSeries,
        enrollee: &Attendee,
        price_cents: i64,
        class_title: &str,
    ) -> Result<RegistrationOutcome> {
        // A priced class must be bounded. Re-checked here (not only in the
        // admin form) because the row could have been written by hand, and
        // selling an unbounded class is the failure this rule exists for.
        crate::domain::validate_pass_pricing(&series.pricing(), series.until_date)
            .map_err(AppError::BadRequest)?;

        if price_cents > MAX_PAYMENT_CENTS {
            return Err(AppError::BadRequest(format!(
                "Class price exceeds the ${} cap on a single payment",
                MAX_PAYMENT_CENTS / 100,
            )));
        }

        let payer = enrollee.as_payer();

        // Double-charge guard, keyed on (series, member) or (series, guest
        // email). Double-clicking and the back button are the realistic
        // ways somebody gets billed twice.
        if let Some(existing) = self
            .payment_repo
            .find_series_pass_payment(series.id, &payer)
            .await?
        {
            match existing.status {
                // Already paid: charge nothing, keep the enrollment.
                PaymentStatus::Completed => return Ok(RegistrationOutcome::Registered),
                // A checkout is in flight: hand back that session rather
                // than minting a second one (and a second enrollment).
                PaymentStatus::Pending => {
                    if let Some(url) = self.in_flight_session_url(&existing.external_id).await {
                        return Ok(RegistrationOutcome::Checkout { url });
                    }
                    return Err(AppError::BadRequest(
                        "A checkout for this class is already in progress. Finish it, or wait \
                         a few minutes for it to expire and try again."
                            .to_string(),
                    ));
                }
                // Refunded (or anything else non-Failed): the enrollment
                // was cancelled with the refund, so this is a fresh
                // purchase and falls through.
                _ => {}
            }
        }

        // Claim FIRST. Race-safe and capacity-enforcing; a full class
        // returns BadRequest before any money is involved.
        self.enrollment_repo
            .claim(series.id, enrollee, series.max_enrollments)
            .await?;

        let runtime = self.stripe_handle.current();
        let Some(stripe_client) = runtime.client.as_ref() else {
            self.release(series.id, enrollee).await;
            return Err(AppError::ServiceUnavailable(
                "Payment processing is not configured".to_string(),
            ));
        };

        let session = stripe_client
            .create_series_pass_checkout_session(
                &payer,
                series.id,
                class_title,
                price_cents,
                self.return_url(series.id, enrollee),
                self.return_url(series.id, enrollee),
            )
            .await;

        let (url, payment_id) = match session {
            Ok(v) => v,
            Err(e) => {
                // The place can never be paid for now, so it must not keep
                // holding capacity.
                self.release(series.id, enrollee).await;
                return Err(e);
            }
        };

        // Link last. An unlinked PendingPayment enrollment still holds its
        // place (see HELD_ENROLLMENT_PREDICATE) — that's what stops two
        // buyers racing for the last place from both winning — so a
        // failure here leaves a held place an admin must release from the
        // class roster. Logged rather than propagated: the session exists
        // and they may already be paying.
        if let Err(e) = self
            .enrollment_repo
            .link_payment(series.id, enrollee, payment_id)
            .await
        {
            tracing::error!(
                "Series {} enrollment for {:?} could not be linked to payment {}: {}",
                series.id,
                enrollee,
                payment_id,
                e,
            );
        }

        Ok(RegistrationOutcome::Checkout { url })
    }

    /// Where Stripe sends the browser back to. A member lands on the
    /// portal's event list; a guest has no portal, so they land back on
    /// the class's public page, which reads their real state from the DB.
    fn return_url(&self, series_id: Uuid, enrollee: &Attendee) -> String {
        match enrollee {
            Attendee::Member(_) => format!("{}/portal/events", self.base_url),
            Attendee::Guest { .. } => {
                format!("{}/classes/{}/register", self.base_url, series_id)
            }
        }
    }

    /// Best-effort release. Already on an error path, so a failure here is
    /// logged rather than replacing the original error.
    async fn release(&self, series_id: Uuid, enrollee: &Attendee) {
        if let Err(e) = self.enrollment_repo.release(series_id, enrollee).await {
            tracing::error!(
                "Failed to release claimed enrollment for series {} {:?}: {}",
                series_id,
                enrollee,
                e,
            );
        }
    }

    /// The hosted URL of an already-open Checkout session, or `None` if it
    /// can't be reached. `None` means "don't reuse it".
    async fn in_flight_session_url(&self, external_id: &Option<StripeRef>) -> Option<String> {
        let Some(StripeRef::CheckoutSession(session_id)) = external_id else {
            return None;
        };
        let runtime = self.stripe_handle.current();
        let client = runtime.client.as_ref()?;
        let session = client
            .gateway()
            .retrieve_checkout_session(session_id)
            .await
            .ok()?;
        session.is_open.then_some(session.url).flatten()
    }
}

// ---------------------------------------------------------------------
// Enrollment ↔ attendance, as free functions.
//
// The completion webhook and the refund path both run these, and the
// webhook dispatcher is constructed by `StripeHandle` before any service
// exists. Taking repositories rather than a service keeps ONE
// implementation of each transition instead of a service copy and a
// dispatcher copy that drift.
// ---------------------------------------------------------------------

/// Seat `enrollee` on every occurrence of `series_id` that has not yet
/// started, linked to `payment_id` when there is one.
///
/// "Not yet started" is tested on the derived UTC instant, not the stored
/// wall-clock, so a non-UTC org's evening session doesn't drop out of the
/// future by the org's offset.
///
/// Attendance created here is deliberately NOT re-checked against the
/// occurrence's `max_attendees`: the place was bought at series scope, and
/// bouncing a pass-holder at session four would be money taken for a seat
/// that does not exist — the exact failure paid events exist to prevent.
pub async fn seat_future_occurrences(
    event_repo: &dyn EventRepository,
    series_id: Uuid,
    enrollee: &Attendee,
    payment_id: Option<Uuid>,
) -> Result<()> {
    let now = chrono::Utc::now();
    for occurrence in event_repo.list_series_occurrences(series_id).await? {
        if occurrence.start_utc() <= now {
            continue;
        }
        event_repo
            .register_attendance(occurrence.id, enrollee)
            .await?;
        if let Some(payment_id) = payment_id {
            event_repo
                .link_payment(occurrence.id, enrollee, payment_id)
                .await?;
        }
    }
    Ok(())
}

/// Confirm the enrollment holding `payment_id` and materialize its
/// attendance. Returns the enrollment when THIS call confirmed it, and
/// `None` when there was nothing to confirm (no such enrollment, or it was
/// already confirmed / cancelled) — which is what makes a Stripe redelivery
/// a no-op.
pub async fn confirm_enrollment_for_payment(
    enrollment_repo: &dyn SeriesEnrollmentRepository,
    event_repo: &dyn EventRepository,
    payment_id: Uuid,
) -> Result<Option<crate::domain::SeriesEnrollment>> {
    if !enrollment_repo.confirm_for_payment(payment_id).await? {
        return Ok(None);
    }
    let Some(enrollment) = enrollment_repo.find_by_payment(payment_id).await? else {
        return Ok(None);
    };
    seat_future_occurrences(
        event_repo,
        enrollment.series_id,
        &enrollment.enrollee,
        Some(payment_id),
    )
    .await?;
    Ok(Some(enrollment))
}

/// Cancel the enrollment holding `payment_id` and its attendance for
/// occurrences that have not yet started. The refund path.
///
/// Attendance for occurrences that already happened is RETAINED: it is a
/// record of who was present, and a later financial event does not get to
/// rewrite it.
pub async fn cancel_enrollment_for_payment(
    enrollment_repo: &dyn SeriesEnrollmentRepository,
    event_repo: &dyn EventRepository,
    payment_id: Uuid,
) -> Result<()> {
    let Some(enrollment) = enrollment_repo.find_by_payment(payment_id).await? else {
        return Ok(());
    };
    enrollment_repo.cancel_for_payment(payment_id).await?;

    let now = chrono::Utc::now();
    for occurrence in event_repo
        .list_series_occurrences(enrollment.series_id)
        .await?
    {
        if occurrence.start_utc() <= now {
            continue;
        }
        event_repo
            .cancel_attendance(occurrence.id, &enrollment.enrollee)
            .await?;
    }
    Ok(())
}

/// The class's name. A series row carries no title of its own — the
/// title lives on each occurrence — so the class is named by any one of
/// them. Falls back to a neutral label for a series whose occurrences
/// have all been removed, so a checkout line item is never blank.
pub async fn class_title(event_repo: &dyn EventRepository, series_id: Uuid) -> String {
    event_repo
        .list_series_occurrences(series_id)
        .await
        .ok()
        .and_then(|occ| occ.into_iter().next())
        .map(|e| e.title)
        .unwrap_or_else(|| "Class".to_string())
}

/// Seat every confirmed enrollee of `series_id` on one occurrence — the
/// horizon roll-forward's hook. Without this an enrollee silently vanishes
/// from sessions materialized after they enrolled.
pub async fn seat_enrollees_on_occurrence(
    enrollment_repo: &dyn SeriesEnrollmentRepository,
    event_repo: &dyn EventRepository,
    series_id: Uuid,
    event_id: Uuid,
) -> Result<()> {
    for enrollment in enrollment_repo.list_confirmed(series_id).await? {
        event_repo
            .register_attendance(event_id, &enrollment.enrollee)
            .await?;
        if let Some(payment_id) = enrollment.payment_id {
            event_repo
                .link_payment(event_id, &enrollment.enrollee, payment_id)
                .await?;
        }
    }
    Ok(())
}
