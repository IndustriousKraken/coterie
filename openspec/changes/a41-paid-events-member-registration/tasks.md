# Tasks

Money path. Every task that claims a seat, mints a session, or flips a payment is
an ordering-sensitive step — review those against `design.md`'s single invariant:
never hold money for a seat that doesn't exist, never hold a seat nobody paid for.

## 1. Storage

- [ ] 1.1 Migration: `ALTER TABLE events ADD COLUMN member_price_cents INTEGER NOT
  NULL DEFAULT 0`. `0` = free, so every existing event backfills to free by the
  default. Do NOT make this nullable — see the spec's rationale on why `NULL` as
  a stand-in for zero breaks equality, range, and aggregate queries.
- [ ] 1.2 Same migration: `ALTER TABLE event_attendance ADD COLUMN payment_id TEXT
  REFERENCES payments(id)`.
- [ ] 1.3 Same migration: widen the `event_attendance.status` CHECK constraint to
  admit `'PendingPayment'`. SQLite requires a table rebuild for a CHECK change —
  do it in the documented 12-step form (new table, copy, drop, rename) inside the
  migration, preserving the composite PK and the `ON DELETE CASCADE` FKs.
- [ ] 1.4 Index `event_attendance(payment_id)` — the refund path looks a seat up
  by payment id.

## 2. Domain

- [ ] 2.1 `src/domain/event.rs`: `member_price_cents: i64` on `Event` (not an
  `Option`), plus an `is_paid_for_members()` predicate returning
  `member_price_cents > 0` so templates and services don't re-derive the rule.
- [ ] 2.2 `src/domain/event.rs`: add `PendingPayment` to `AttendanceStatus` and a
  `payment_id: Option<Uuid>` field on `EventAttendance`.
- [ ] 2.3 `src/domain/payment.rs`: add `PaymentKind::EventFee { event_id: Uuid }`;
  extend `as_str()`/parse with `"event_fee"`. The compiler will surface every
  match site — handle each explicitly rather than adding a catch-all arm.
- [ ] 2.4 Update `PaymentKind::Other`'s doc comment: it currently names "event
  fees" as its example, which is now wrong.

## 3. Repository

- [ ] 3.1 `EventRepository::count_held_seats(event_id)` — the join from
  `design.md`: `Registered`, plus `PendingPayment` whose payment is still
  `Pending`.
- [ ] 3.2 `EventRepository::claim_seat(event_id, member_id, max_attendees)` —
  count-and-insert in ONE transaction, returning a typed "full" error. This is the
  race-safety point; a count outside the transaction is the bug this task exists
  to prevent.
- [ ] 3.3 `EventRepository::link_payment(event_id, member_id, payment_id)`,
  `confirm_seat(payment_id)`, `release_seat(event_id, member_id)`, and
  `cancel_seat_for_payment(payment_id)`.
- [ ] 3.4 `EventRepository::roster(event_id)` — attendee + attendance status +
  joined payment status/method for the admin view.
- [ ] 3.5 `find_event_fee_payment(event_id, member_id)` on the payment repo for
  the double-charge guard, and `list_completed_event_fees(event_id)` for the
  delete-refund sweep.

## 4. Registration service

- [ ] 4.1 `src/service/event_registration_service.rs`: `register(member, event)`
  implementing the ordered flow — double-charge guard, claim seat, create payment
  + Checkout session, release seat on failure.
- [ ] 4.2 Free events short-circuit to today's `register_attendance` before any
  payment machinery is touched.
- [ ] 4.3 Reject registration by a member who is not Active/Honorary (matches the
  existing RSVP rule) before claiming a seat.
- [ ] 4.4 Validate `member_price_cents` on the way in: a stored price is `> 0`
  and `<= MAX_PAYMENT_CENTS`.

## 5. Stripe + webhooks

- [ ] 5.1 `src/payments/stripe_client.rs`: `create_event_fee_checkout_session` —
  metadata `payment_type=event_fee` + `event_id`, line-item named for the event,
  `expires_at` at 60 minutes (bounds an abandoned seat).
- [ ] 5.2 `webhook_dispatcher/checkout.rs`: add the `event_fee` branch to
  `handle_successful_payment` — flip payment `Completed`, confirm the seat. Must
  NOT extend dues or reschedule auto-renew. Guard the side effects behind the
  `won_flip` result exactly as the donation branch does, so a Stripe retry is a
  no-op.
- [ ] 5.3 Confirm `handle_expired_session` needs no change: the seat-count query
  already stops counting a seat whose payment left `Pending`. Add a test that
  proves it rather than assuming it.
- [ ] 5.4 `webhook_dispatcher/charge.rs`: on `charge.refunded` for an `EventFee`
  payment, cancel the linked seat.
- [ ] 5.5 Audit the money transitions: seat confirmed on completion (once, inside
  the `won_flip` guard so a redelivery doesn't double-log), refund, at-the-door,
  comp, stuck-seat release, and the delete-time bulk refund. Claiming a
  `PendingPayment` seat is deliberately NOT audited — no money has moved.

## 6. Web surface

- [ ] 6.1 `src/web/portal/events.rs`: paid branch in `rsvp_event` returning an
  HTMX redirect to Checkout; button label shows the price.
- [ ] 6.2 Admin event create/update forms + handlers: the price field. A blank
  field and a typed `0` both store `0` — no error either way, and no rewriting to
  `NULL`. Reject only negatives and over-cap values.
- [ ] 6.3 Admin event detail: the roster (status + payment state), at-the-door and
  comp actions via `PaymentService::record_manual`, and release-stuck-seat. All
  admin-auth + CSRF + audited.
- [ ] 6.4 `admin_delete_event`: refund all `Completed` event-fee payments BEFORE
  deleting; abort the delete and surface the error if any refund fails.
- [ ] 6.5 Receipts name the event for an `EventFee` payment.

## 7. Tests

- [ ] 7.1 Race: two concurrent registrations for the last seat — exactly one
  reaches Checkout, the other gets `BadRequest` with no payment row.
- [ ] 7.2 Abandoned checkout frees the seat (expire the session, register again,
  succeed).
- [ ] 7.3 Double-charge guard: re-register with a `Completed` payment charges
  nothing; with a `Pending` payment returns the same session.
- [ ] 7.4 Webhook completion is idempotent under redelivery, and does not move
  `dues_paid_until`.
- [ ] 7.5 Refund (both admin route and `charge.refunded`) cancels the seat.
- [ ] 7.6 Deleting a paid event refunds attendees; a failing refund aborts the
  delete and leaves the roster intact.
- [ ] 7.7 Free events are unchanged: no payment row, no session, immediate
  `Registered`, capacity still advisory.
- [ ] 7.8 Audit: `manual_event_fee` and `waive_event_fee` action strings, and the
  pre-existing `waive_dues` mapping still holds for membership.
