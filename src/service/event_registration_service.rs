//! Registration for an event — the seat/charge/release state machine
//! behind `POST /portal/api/events/:id/rsvp` (members) and
//! `POST /public/events/:id/register` (guests).
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
//! A guest seat is the same seat with a different payer: it runs the
//! same ordering through the same [`Self::hold_and_charge`], and is
//! released and confirmed by the same payment-status transitions. There
//! is deliberately no second state machine — a second one would drift
//! from the first on exactly the double-pay / race / refund edges that
//! cost real money.
//!
//! Free member events never touch any of this — they short-circuit to
//! the same `register_attendance` upsert they used before paid events
//! existed, with capacity still advisory. A free *guest* registration
//! claims through the capacity guard first, because a public door with
//! no capacity check is a roster nobody can trust.

use std::sync::Arc;

use uuid::Uuid;

use crate::{
    domain::{Attendee, Event, Member, MemberStatus, PaymentStatus, StripeRef, MAX_PAYMENT_CENTS},
    error::{AppError, Result},
    payments::StripeHandle,
    repository::{EventRepository, PaymentRepository},
};

/// Longest guest name / email accepted on the public endpoint. Mirrors
/// the caps `/public/signup` and `/public/donate` already apply to the
/// same two fields — this is unauthenticated input, so it is bounded at
/// the boundary rather than at the column.
const MAX_GUEST_NAME: usize = 200;
const MAX_GUEST_EMAIL: usize = 254;

/// What the caller should do with the registrant next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrationOutcome {
    /// Seat is confirmed — free event, or they already paid.
    Registered,
    /// Send them to Stripe. The seat is held as `PendingPayment`
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

        let attendee = Attendee::Member(member.id);

        // Free event: today's behavior, untouched. No payment row, no
        // session, capacity still advisory.
        if !event.is_paid_for_members() {
            self.event_repo
                .register_attendance(event.id, &attendee)
                .await?;
            return Ok(RegistrationOutcome::Registered);
        }

        self.hold_and_charge(event, &attendee, event.member_price_cents)
            .await
    }

    /// Register a guest (non-member) for `event` at the guest price.
    ///
    /// The supplied email is NEVER looked up against the member
    /// directory: this is unverified input from an unauthenticated
    /// caller, and matching it would let anyone write an attendance row
    /// and a payment row into a named member's account, and pick that
    /// member's price bracket, by typing their address. A guest
    /// registration stays a guest registration even when the email
    /// matches a member exactly.
    ///
    /// Callers are responsible for the protections in front of this
    /// (rate limit, then bot challenge) and for the `publicly_registerable`
    /// check — this method is the money, not the door.
    pub async fn register_guest(
        &self,
        event: &Event,
        name: &str,
        email: &str,
    ) -> Result<RegistrationOutcome> {
        let attendee = guest_attendee(name, email)?;

        if !event.is_paid_for_guests() {
            return self.register_free_guest(event, &attendee).await;
        }

        self.hold_and_charge(event, &attendee, event.guest_price_cents)
            .await
    }

    /// Free public registration: claim the seat through the capacity
    /// guard, then confirm it immediately. No payment row and no
    /// Checkout session — the same short-circuit a free member
    /// registration takes, with the capacity check kept because a free
    /// public door is exactly where seat-squatting shows up.
    ///
    /// There is no payment row to deduplicate against here, so the
    /// already-seated check plus the `UNIQUE(event_id, guest_email)`
    /// constraint are what stop a double booking.
    async fn register_free_guest(
        &self,
        event: &Event,
        attendee: &Attendee,
    ) -> Result<RegistrationOutcome> {
        use crate::domain::AttendanceStatus;

        // Re-submitting returns the existing seat rather than resetting
        // it — claiming again would drop a confirmed seat back to
        // `PendingPayment` and hand out a second confirmation.
        if let Some(AttendanceStatus::Registered) = self
            .event_repo
            .attendance_status(event.id, attendee)
            .await?
        {
            return Ok(RegistrationOutcome::Registered);
        }

        // Claim (capacity-enforcing) then confirm. Two statements rather
        // than one because the claim is what serializes the race; the
        // upsert to `Registered` is what a41's free member path already
        // does.
        //
        // ponytail: if the second call fails the row stays
        // `PendingPayment` with no payment and holds its seat until an
        // admin releases it from the roster — the same bounded, visible
        // ceiling as a41's unlinked claim. A compensating delete here
        // would be a second failure path to get wrong.
        self.event_repo
            .claim_seat(event.id, attendee, event.max_attendees)
            .await?;
        self.event_repo
            .register_attendance(event.id, attendee)
            .await?;
        Ok(RegistrationOutcome::Registered)
    }

    /// The paid path, shared by members and guests: double-charge guard
    /// → claim the seat → mint the Checkout session → link the payment,
    /// releasing the claim if the session can't be created.
    async fn hold_and_charge(
        &self,
        event: &Event,
        attendee: &Attendee,
        price_cents: i64,
    ) -> Result<RegistrationOutcome> {
        // The caller only reaches here for a price > 0; the cap is
        // re-checked because the row could predate the form validation
        // (or have been written by hand).
        if price_cents > MAX_PAYMENT_CENTS {
            return Err(AppError::BadRequest(format!(
                "Event price exceeds the ${} cap on a single payment",
                MAX_PAYMENT_CENTS / 100,
            )));
        }

        let payer = attendee.as_payer();

        // Double-charge guard, keyed on (event, member) or (event, guest
        // email). Double-clicking the button and using the back button
        // are the realistic ways somebody gets billed twice.
        if let Some(existing) = self
            .payment_repo
            .find_event_fee_payment(event.id, &payer)
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
        // Guest and member claims contend for the same rows.
        self.event_repo
            .claim_seat(event.id, attendee, event.max_attendees)
            .await?;

        let runtime = self.stripe_handle.current();
        let Some(stripe_client) = runtime.client.as_ref() else {
            self.release(event.id, attendee).await;
            return Err(AppError::ServiceUnavailable(
                "Payment processing is not configured".to_string(),
            ));
        };

        let session = stripe_client
            .create_event_fee_checkout_session(
                &payer,
                event.id,
                &event.title,
                price_cents,
                self.return_url(event, attendee),
                self.return_url(event, attendee),
            )
            .await;

        let (url, payment_id) = match session {
            Ok(v) => v,
            Err(e) => {
                // The seat can never be paid for now, so it must not
                // keep holding capacity.
                self.release(event.id, attendee).await;
                return Err(e);
            }
        };

        // Link last. An unlinked PendingPayment row still holds its seat
        // (see HELD_SEAT_PREDICATE) — that's what stops two registrants
        // racing for the last seat from both winning — so a failure here
        // leaves a held seat an admin must release from the roster.
        // Logged rather than propagated: the session exists and they
        // may already be paying, so failing the call now would be worse
        // than a stuck row.
        if let Err(e) = self
            .event_repo
            .link_payment(event.id, attendee, payment_id)
            .await
        {
            tracing::error!(
                "Event {} seat for {:?} could not be linked to payment {}: {}",
                event.id,
                attendee,
                payment_id,
                e,
            );
        }

        Ok(RegistrationOutcome::Checkout { url })
    }

    /// Where Stripe sends the browser back to. A member lands on the
    /// portal's event list as before; a guest has no portal, so they
    /// land back on the event's public registration page, which reads
    /// their seat's real state from the database.
    fn return_url(&self, event: &Event, attendee: &Attendee) -> String {
        match attendee {
            Attendee::Member(_) => format!("{}/portal/events", self.base_url),
            Attendee::Guest { .. } => {
                format!("{}/events/{}/register", self.base_url, event.id)
            }
        }
    }

    /// Best-effort seat release. Already on an error path, so a failure
    /// here is logged rather than replacing the original error — which
    /// is the one the registrant and the operator need to see.
    async fn release(&self, event_id: Uuid, attendee: &Attendee) {
        if let Err(e) = self.event_repo.release_seat(event_id, attendee).await {
            tracing::error!(
                "Failed to release claimed seat for event {} {:?}: {}",
                event_id,
                attendee,
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

/// Validate and bound a guest identity at the boundary, before it
/// reaches a seat or a payment row. Unauthenticated free text is a
/// storage-abuse vector, and an unusable email means a confirmed seat
/// whose holder never hears about it — the caps mirror
/// `/public/signup` and `/public/donate`.
pub fn guest_attendee(name: &str, email: &str) -> Result<Attendee> {
    let name = name.trim();
    let email = email.trim();
    if email.is_empty() || !email.contains('@') {
        return Err(AppError::BadRequest("Valid email is required".to_string()));
    }
    if email.len() > MAX_GUEST_EMAIL {
        return Err(AppError::BadRequest("Email too long".to_string()));
    }
    if name.is_empty() {
        return Err(AppError::BadRequest("Name is required".to_string()));
    }
    if name.len() > MAX_GUEST_NAME {
        return Err(AppError::BadRequest("Name too long".to_string()));
    }
    Ok(Attendee::Guest {
        name: name.to_string(),
        email: email.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_identity_is_trimmed_and_bounded() {
        let a = guest_attendee("  Ada Lovelace  ", " ada@example.com ").unwrap();
        assert_eq!(
            a,
            Attendee::Guest {
                name: "Ada Lovelace".to_string(),
                email: "ada@example.com".to_string(),
            }
        );

        // A guest is a non-member payer, always — even if the email
        // happens to match a member, which this layer never checks.
        assert_eq!(a.member_id(), None);
        assert_eq!(a.as_payer().member_id(), None);

        for (name, email) in [
            ("Ada", ""),
            ("Ada", "not-an-email"),
            ("", "ada@example.com"),
            ("   ", "ada@example.com"),
            (
                "Ada",
                &format!("{}@example.com", "a".repeat(MAX_GUEST_EMAIL)),
            ),
            (&"n".repeat(MAX_GUEST_NAME + 1), "ada@example.com"),
        ] {
            assert!(
                guest_attendee(name, email).is_err(),
                "name={name:?} email={email:?} should be rejected",
            );
        }
    }
}
