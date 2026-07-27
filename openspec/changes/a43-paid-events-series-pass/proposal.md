# a43-paid-events-series-pass

## Why

`a41` sells a seat at one event and `a42` opens that to the public. Neither can
sell a **class** — "Intro to Lockpicking, six Tuesdays, $120" — which is the shape
most orgs actually charge for. Under a41 alone the organizer has to price each of
the six occurrences separately and hope every attendee buys all six; nobody
running a class wants to sell it that way, and nobody taking one wants to check
out six times.

This change adds a **series pass**: one payment enrolls the attendee in every
remaining session of a recurring series. It is the last piece of the paid-events
arc and reuses a41's money machinery unchanged — the pass is one payment, and
enrollment is what that payment buys.

## What Changes

- **Pass pricing on the series.** `event_series` gains `member_price_cents` and
  `guest_price_cents` (both `NOT NULL DEFAULT 0`, zero means free) plus
  `guest_registration_enabled`, matching the column semantics a41/a42 established
  on `events`. A series with a pass price is a **paid class**.
- **A paid class must be bounded.** A pass price is only permitted on a series
  with an `until_date`. Selling an open-ended series for one flat price is a
  subscription, not a pass, and Coterie already has recurring billing for
  subscriptions.
- **`series_enrollment`** records who bought the pass — member or guest identity,
  the linked payment, and status — using the same `PendingPayment` → confirmed →
  cancelled lifecycle a41 defined for a single seat.
- **`PaymentKind::SeriesPass { series_id }`**, sibling to `EventFee { event_id }`,
  so a refund can find the enrollment and so class revenue is separable in
  reporting.
- **Enrollment materializes attendance** rows for every future occurrence, so
  rosters, check-in, reminders, and iCal keep working per-occurrence with no new
  read paths. The horizon roll-forward extends an active enrollment onto newly
  materialized occurrences.
- **Capacity is a series-level number** for a paid class — twelve seats in the
  class, not twelve seats in each of six nights.
- **Flat pricing, explicitly.** Joining late costs full price; cancelling refunds
  in full regardless of sessions already held. No proration.
- **Cancelling one occurrence is not a refund event** — a holiday skip is normal
  and does not entitle anyone to money back.

## Impact

- **Spec:** `paid-events` gains 9 ADDED requirements. MODIFIED: `domain-types`
  (the `PaymentKind` variant set gains `SeriesPass`), `payment-recording` (the
  `record_manual` audit mapping gains series-pass arms, and the four-entry-points
  requirement carries a41's placeholder amendment).
- **On the repeated `payment-recording` amendment:** like a42, this change
  restates a41's distinction between *recording* a payment and *opening* a
  `Pending` placeholder rather than depending on a41 having archived first. A
  series-pass checkout writes a placeholder on the same basis an event-fee
  checkout does, so the amendment must be in scope for this change to be
  self-consistent against canon in any evaluation order.
- **Code (new):** migration adding the series pricing columns and the
  `series_enrollment` table; `SeriesEnrollmentService` wrapping a41's seat
  machinery at series scope; a series-level roster and enrollment UI.
- **Code (extend):** `RecurringEventService` horizon roll-forward materializes
  attendance for active enrollments; `stripe_client` gains a series-pass Checkout
  session; the webhook `event_fee` branch generalizes to cover `series_pass`;
  the series delete path refunds before deleting.
- **Reuse:** a41's seat-claim ordering, payment-status-driven release, refund
  handling, audit rules, and roster actions; a42's guest identity and public
  registration surface where the class is open to non-members.
- **Behavior for orgs that never price a series:** none. Every series is free
  until someone sets a pass price.
- **Deferred:** proration and partial refunds, drop-in pricing for a single night
  of a paid class (an org wanting both should price the occurrences and the pass
  independently — the general form of that is its own change), transferring a
  pass between people, and per-session make-up credits.
