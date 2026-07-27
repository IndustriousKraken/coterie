# Tasks

Depends on `a41-paid-events-member-registration` (the seat lifecycle) and, for
guest enrollment, `a42-paid-events-guest-registration` (guest identity + the
public surface). Reuse both; do not fork a third state machine.

## 1. Storage

- [x] 1.1 Migration: `event_series` gains `member_price_cents INTEGER NOT NULL
  DEFAULT 0`, `guest_price_cents INTEGER NOT NULL DEFAULT 0`, and
  `guest_registration_enabled BOOLEAN NOT NULL DEFAULT 0`. Zero means free —
  never `NULL`, for the reasons a41's spec records.
- [x] 1.2 Same migration: `event_series` gains a capacity column for the class
  (nullable is correct here — absent genuinely means "no limit", not "zero").
- [x] 1.3 Same migration: create `series_enrollment` — surrogate `id`, `series_id`
  FK, nullable `member_id`, guest identity columns, `payment_id`, `status`,
  `enrolled_at`. Reuse a42's exactly-one-identity CHECK shape.
- [x] 1.4 `UNIQUE(series_id, member_id)` and `UNIQUE(series_id, guest_email)` so
  one-enrollment-per-identity is a database guarantee, not just a service check.

## 2. Domain

- [x] 2.1 `PaymentKind::SeriesPass { series_id: Uuid }` + `"series_pass"` wire
  string. Handle every match site the compiler surfaces explicitly; no catch-all.
- [x] 2.2 `EventSeries`: the pricing/capacity fields plus an `is_paid_class()`
  predicate (`member_price_cents > 0`).
- [x] 2.3 `SeriesEnrollment` domain type reusing a42's identity sum type rather
  than a fresh set of loose optionals.

## 3. Validation

- [x] 3.1 Reject a pass price on a series whose `until_date` is `NULL`, with a
  message naming the missing end date. This is the guard that stops an operator
  selling an unbounded class for one flat price.
- [x] 3.2 Price bounds identical to a41: negatives and over-cap rejected, zero
  accepted and stored as zero.

## 4. Enrollment service

- [x] 4.1 `SeriesEnrollmentService::enroll` — double-enrollment guard, claim
  enrollment in a transaction against series capacity, create the `SeriesPass`
  payment + Checkout session, release the claim on failure. Same ordering as a41.
- [x] 4.2 On confirmation, materialize `event_attendance` rows for every occurrence
  that has not yet started. Do NOT back-fill occurrences that already started.
- [x] 4.3 Attendance rows created from an enrollment bypass per-occurrence
  `max_attendees` — the seat was already bought at series scope.
- [x] 4.4 Free classes (`member_price_cents = 0`) short-circuit to enrollment with
  no payment machinery, mirroring a41's free-event path.

## 5. Recurring-service integration

- [x] 5.1 Horizon roll-forward: when a new occurrence materializes for a series
  with active enrollments, create attendance rows for those enrollees. Without
  this, enrollees silently vanish from later sessions.
- [x] 5.2 Occurrence cancellation (a35 exceptions) cancels only that occurrence's
  attendance rows — no refund, no enrollment change.
- [x] 5.3 Series delete refunds every `Completed` series-pass payment BEFORE
  deleting, aborting on any refund failure.

## 6. Stripe + webhooks

- [x] 6.1 `create_series_pass_checkout_session` — metadata `payment_type=series_pass`
  + `series_id`, line item named for the class, same 60-minute expiry as a41.
- [x] 6.2 Generalize the webhook completion branch to handle `series_pass`
  alongside `event_fee`: flip the payment, confirm the enrollment, materialize
  attendance. Must not extend dues. Guard side effects behind the `won_flip`
  result so a Stripe retry is a no-op.
- [x] 6.3 `charge.refunded` for a `SeriesPass` payment cancels the enrollment and
  its future attendance rows, leaving past ones intact.

## 7. Web surface

- [x] 7.1 Admin series form: pass prices, class capacity, guest-enrollment toggle.
- [x] 7.2 Admin series detail: enrollment roster with payment state; at-the-door,
  comp, release-stuck, refund — all audited.
- [x] 7.3 Member portal: enroll control on a paid class showing the pass price.
- [x] 7.4 Public class registration page where guest enrollment is enabled,
  reusing a42's page, protections, and 404 rules.
- [x] 7.5 Receipts name the class for a `SeriesPass` payment.

## 8. Tests

- [x] 8.1 Pass price on an unbounded series is rejected.
- [x] 8.2 Confirmed pass creates attendance for all future occurrences and none
  for already-started ones.
- [x] 8.3 Roll-forward adds attendance for a newly materialized occurrence of an
  enrolled series.
- [x] 8.4 Series capacity is race-safe; concurrent last-place enrollment yields
  exactly one winner.
- [x] 8.5 A pass-holder is never bounced by a per-occurrence `max_attendees`.
- [x] 8.6 Late enrollee pays full price; mid-class refund returns the full amount.
- [x] 8.7 Refund cancels future attendance and preserves past attendance.
- [x] 8.8 Cancelling one occurrence refunds nobody and cancels no enrollment.
- [x] 8.9 Series delete refunds all enrollees; a failing refund aborts the delete.
- [x] 8.10 Audit strings `manual_series_pass` / `waive_series_pass`, and the
  pre-existing `waive_dues` and `waive_event_fee` mappings still hold.
