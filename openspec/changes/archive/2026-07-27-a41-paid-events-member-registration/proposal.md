# a41-paid-events-member-registration

## Why

Coterie can bill dues and accept donations, but it cannot charge for a single
event. An org running a paid workshop, a ticketed talk, or a class today has to
collect money out-of-band (Venmo, cash, a separate Eventbrite) and reconcile the
attendee list by hand. `PaymentKind::Other` even names "event fees" in its doc
comment, but nothing wires an event to a payment.

This change adds the smallest complete money loop: **an admin puts a price on an
event, a member registers, pays through Stripe Checkout, and lands on the roster
as paid.** It is deliberately members-only and single-event — guest (non-member)
registration and multi-week class passes are follow-on changes that build on the
state machine this one establishes.

It is a **change**, not an issue: it adds net-new behavior with new persisted
state (a price on the event, a payment link and a payment-pending state on the
attendance row).

### Why capacity enforcement is in scope

`EventRepository::register_attendance` is today a bare upsert with **no capacity
check** — `max_attendees` is advisory, and the `Waitlisted` status is unused. For
a free RSVP an oversell is a mild annoyance. For a paid seat it is **taking money
for a seat that does not exist**, which is a refund incident and a trust problem.
So paid events enforce capacity, and the enforcement is race-safe: the seat is
claimed *before* the Stripe session is created, never after.

Free events keep today's advisory behavior — tightening them is a separate
concern and is not smuggled in here.

### Why no fifth payment entry point

`payment-recording` enumerates exactly four permitted payment-writing entry
points and forbids ad-hoc `payment_repo.create` calls. This change adds **none**.
Event-fee money flows through two entry points that already exist:

- **`WebhookDispatcher::handle_*`** — the Stripe Checkout completion path, which
  already branches on the `payment_type` session-metadata key
  (`membership` vs `donation`). This change adds an `event_fee` branch.
- **`PaymentService::record_manual`** — for at-the-door cash (`Manual`) and
  comped seats (`Waived`).

Seat release on abandonment is likewise free: `handle_expired_session` already
flips an abandoned checkout's payment to `Failed`, and the seat-count query is
written so a non-`Pending` payment stops holding its seat.

Registration does write a `Pending` placeholder row when it creates the Checkout
session — but so do the membership-checkout, donation-checkout, and saved-card
donation flows today (`stripe_client.rs`, `portal/payments/checkout.rs`,
`portal/donations.rs`). The four-entry-point requirement never named that
practice, so this change amends it to say plainly what was already true: the four
entry points govern *recording* a payment, while *opening* a pending placeholder
before any money moves is a distinct, side-effect-free act that settles into a
recorded payment through the webhook. That closes a documentation gap in canon
rather than widening the rule.

## What Changes

- **Price on the event.** New `member_price_cents` column, `NOT NULL DEFAULT 0`,
  where `0` means free and existing events are backfilled to `0` — no behavior
  change for any org that never sets a price. Set on the existing admin event
  create/update forms. Zero is stored as zero rather than as `NULL`: `NULL` means
  "unknown", and using it as a sentinel for free would silently break
  `WHERE member_price_cents = 0`, range filters, and aggregates.
- **Attendance gains a payment link and an in-flight state.** New
  `PendingPayment` variant on `AttendanceStatus` plus a nullable `payment_id` on
  `event_attendance`. A paid registration is a seat held while the member is at
  Stripe.
- **`PaymentKind::EventFee { event_id }`** — a first-class kind rather than
  untyped `Other`, so event revenue is separable in the billing dashboard,
  receipts name the event, and a refund can find its seat.
- **Registration routes through Checkout when the event is paid.**
  `POST /portal/api/events/:id/rsvp` on a free event behaves exactly as today; on
  a paid event it claims a seat, creates a Checkout session stamped
  `payment_type=event_fee`, and hands back the redirect.
- **Capacity is enforced for paid events** against confirmed *and* in-flight
  seats, race-safely, with no waitlist (deferred).
- **Refund releases the seat**; **deleting a paid event refunds every paid
  attendee first**, and refuses to delete if any refund fails rather than
  stranding charged members.
- **Admin roster** per paid event showing each attendee's payment state, with
  at-the-door / comp recording and a control to release a stuck pending seat.

## Impact

- **Spec:** new capability `paid-events` (8 ADDED requirements). MODIFIED:
  `domain-types` (the `PaymentKind` variant set and its DB string mapping),
  `payment-recording` (the `record_manual` audit-action mapping gains event-fee
  arms), `member-content` (RSVP on a paid event is a checkout, and paid
  registration *is* audited where free RSVP still is not).
- **Code (new):** migration adding `events.member_price_cents`,
  `event_attendance.payment_id`, and `PendingPayment` to the attendance status
  CHECK; `src/service/event_registration_service.rs` holding the seat/charge/
  release state machine; an admin roster view.
- **Code (extend):** `src/domain/{event,payment}.rs` (price field, `EventFee`
  variant, `PendingPayment` status); `src/repository/event_repository.rs`
  (capacity-aware seat claim, roster query, seat release);
  `src/payments/stripe_client.rs` (an event-fee Checkout session);
  `src/payments/webhook_dispatcher/checkout.rs` (`event_fee` branch on complete,
  seat release on expire); `charge.rs` (seat release on refund);
  `src/web/portal/events.rs` (paid RSVP path); admin event form + delete handler.
- **Reuse:** existing Checkout creation, webhook idempotency claim, refund
  handler, audit log, receipts, CSRF layer, `MAX_PAYMENT_CENTS` bound.
- **Behavior for orgs that never set a price:** none. Every event is free until
  someone types a number into the new field.
- **Deferred:** guest/non-member registration and guest pricing
  (`paid-events-guest-registration`), multi-week class passes
  (`paid-events-series-pass`), the marketing-site register button
  (`paid-events-register-link`), waitlists on paid events, partial refunds,
  discount codes, and any sales-tax treatment (event fees are not donations and
  may be taxable — an org needing that should get a real answer, not an
  accidental one).
