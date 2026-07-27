//! Member registration for an event — the seat/charge/release state
//! machine behind `POST /portal/api/events/:id/rsvp`.
//!
//! The one invariant: **never hold money for a seat that does not
//! exist, and never hold a seat nobody paid for.** Everything here
//! falls out of that:
//!
//!   - the seat is claimed BEFORE the Checkout session is created, so a
//!     full event can never mint a session and turn a rejected click
//!     into a refund incident;
//!   - if the session can't be created, the claim is released;
//!   - the seat is confirmed by the webhook, never by the browser's
//!     return to `success_url`, which carries no trust.
//!
//! Free events never touch any of this — they short-circuit to the
//! same `register_attendance` upsert they used before paid events
//! existed, with capacity still advisory.

use std::sync::Arc;

use uuid::Uuid;

use crate::{
    domain::{Event, Member, MemberStatus, PaymentStatus, StripeRef, MAX_PAYMENT_CENTS},
    error::{AppError, Result},
    payments::StripeHandle,
    repository::{EventRepository, PaymentRepository},
};

/// What the caller should do with the member next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrationOutcome {
    /// Seat is confirmed — free event, or the member already paid.
    Registered,
    /// Send the member to Stripe. The seat is held as `PendingPayment`
    /// until the completion webhook confirms it.
    Checkout { url: String },
}

pub struct EventRegistrationService {
    event_repo: Arc<dyn EventRepository>,
    payment_repo: Arc<dyn PaymentRepository>,
    /// Read `current()` per call so a portal key rotation reaches this
    /// path without a restart, same as every other money path.
    stripe_handle: Arc<StripeHandle>,
    base_url: String,
}

impl EventRegistrationService {
    pub fn new(
        event_repo: Arc<dyn EventRepository>,
        payment_repo: Arc<dyn PaymentRepository>,
        stripe_handle: Arc<StripeHandle>,
        base_url: String,
    ) -> Self {
        Self {
            event_repo,
            payment_repo,
            stripe_handle,
            base_url,
        }
    }

    /// Register `member` for `event`, charging when the event has a
    /// member price. See the module docs for the ordering rationale.
    pub async fn register(&self, member: &Member, event: &Event) -> Result<RegistrationOutcome> {
        // Matches the existing RSVP rule (POLICY_REQUIRE_AUTH). Checked
        // here as well as in middleware so a future non-portal caller
        // can't claim a seat for an expired member.
        if !matches!(member.status, MemberStatus::Active | MemberStatus::Honorary) {
            return Err(AppError::BadRequest(
                "Only active members can register for events".to_string(),
            ));
        }

        // Free event: today's behavior, untouched. No payment row, no
        // session, capacity still advisory.
        if !event.is_paid_for_members() {
            self.event_repo
                .register_attendance(event.id, member.id)
                .await?;
            return Ok(RegistrationOutcome::Registered);
        }

        // A stored price is > 0 by `is_paid_for_members`; the cap is
        // re-checked here because the row could predate the form
        // validation (or have been written by hand).
        if event.member_price_cents > MAX_PAYMENT_CENTS {
            return Err(AppError::BadRequest(format!(
                "Event price exceeds the ${} cap on a single payment",
                MAX_PAYMENT_CENTS / 100,
            )));
        }

        // Double-charge guard. Double-clicking the button and using the
        // back button are the realistic ways a member gets billed twice.
        if let Some(existing) = self
            .payment_repo
            .find_event_fee_payment(event.id, member.id)
            .await?
        {
            match existing.status {
                // Already paid: charge nothing, keep the seat.
                PaymentStatus::Completed => return Ok(RegistrationOutcome::Registered),
                // A checkout is in flight: hand back that session
                // rather than minting a second one (and a second seat).
                PaymentStatus::Pending => {
                    if let Some(url) = self.in_flight_session_url(&existing.external_id).await {
                        return Ok(RegistrationOutcome::Checkout { url });
                    }
                    return Err(AppError::BadRequest(
                        "A checkout for this event is already in progress. Finish it, or wait \
                         a few minutes for it to expire and try again."
                            .to_string(),
                    ));
                }
                // Refunded (or anything else non-Failed): the seat was
                // cancelled with the refund, so registering again is a
                // fresh purchase and falls through.
                _ => {}
            }
        }

        // Claim the seat FIRST. Race-safe and capacity-enforcing; a
        // full event returns BadRequest before any money is involved.
        self.event_repo
            .claim_seat(event.id, member.id, event.max_attendees)
            .await?;

        let runtime = self.stripe_handle.current();
        let Some(stripe_client) = runtime.client.as_ref() else {
            self.release(event.id, member.id).await;
            return Err(AppError::ServiceUnavailable(
                "Payment processing is not configured".to_string(),
            ));
        };

        let session = stripe_client
            .create_event_fee_checkout_session(
                member.id,
                event.id,
                &event.title,
                event.member_price_cents,
                format!("{}/portal/events", self.base_url),
                format!("{}/portal/events", self.base_url),
            )
            .await;

        let (url, payment_id) = match session {
            Ok(v) => v,
            Err(e) => {
                // The seat can never be paid for now, so it must not
                // keep holding capacity.
                self.release(event.id, member.id).await;
                return Err(e);
            }
        };

        // Link last. An unlinked PendingPayment row still holds its seat
        // (see HELD_SEAT_PREDICATE) — that's what stops two members
        // racing for the last seat from both winning — so a failure here
        // leaves a held seat an admin must release from the roster.
        // Logged rather than propagated: the session exists and the
        // member may already be paying, so failing the call now would be
        // worse than a stuck row.
        if let Err(e) = self
            .event_repo
            .link_payment(event.id, member.id, payment_id)
            .await
        {
            tracing::error!(
                "Event {} seat for member {} could not be linked to payment {}: {}",
                event.id,
                member.id,
                payment_id,
                e,
            );
        }

        Ok(RegistrationOutcome::Checkout { url })
    }

    /// Best-effort seat release. Already on an error path, so a failure
    /// here is logged rather than replacing the original error — which
    /// is the one the member and the operator need to see.
    async fn release(&self, event_id: Uuid, member_id: Uuid) {
        if let Err(e) = self.event_repo.release_seat(event_id, member_id).await {
            tracing::error!(
                "Failed to release claimed seat for event {} member {}: {}",
                event_id,
                member_id,
                e,
            );
        }
    }

    /// The hosted URL of an already-open Checkout session, or `None` if
    /// it can't be reached (no Stripe ref, Stripe unreachable, session
    /// no longer open). `None` means "don't reuse it".
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
