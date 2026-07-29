# Refuse to sell a class pass when no session remains

## Why

A class pass buys `event_attendance` rows for every occurrence that has
not yet started
(`src/service/series_enrollment_service.rs:320-341`,
`seat_future_occurrences`). When every occurrence has already started,
that loop `continue`s on all of them and creates **zero** rows. Nothing
stops the sale from happening anyway:

- `src/api/handlers/public.rs:1599-1614` — `enroll_in_class` loads the
  series via `load_enrollable`, takes a title, and calls
  `enroll_guest`. `load_enrollable`
  (`src/web/templates/class_register.rs:153-173`) tests only
  `publicly_enrollable` — guest enrollment on, occurrences `Public`. There
  is no "are there sessions left" test.
- `src/web/portal/events.rs:486-513` — `enroll_in_series` loads the series
  by id and calls `enroll`. Same gap.

The Coterie-hosted class page already knows the answer and hides the form:
`ClassRegisterTemplate::is_over` is set from
`upcoming.is_empty()` (`src/web/templates/class_register.rs:70,140`), and
the member portal only renders the enroll control for a non-past
occurrence (`src/web/portal/events.rs:144`). Both are page-level
decisions; the endpoints are the trust boundary and enforce neither. A
bookmarked URL, a stale tab, a marketing-site form, or a direct POST
reaches them.

Result: the buyer is charged the full guest or member pass price, the
completion webhook confirms the enrollment
(`src/payments/webhook_dispatcher/checkout.rs:138-198`), and
`seat_future_occurrences` seats them on nothing. Money taken for zero
sessions — the failure the paid-events capability names as the one it
exists to prevent ("money taken for a seat that does not exist",
`openspec/specs/paid-events/spec.md:817`).

**This is a contract change**, which is why it is a spec-lane change
rather than an issue. Canon currently *permits* the behavior: the
requirement "Pass pricing is flat, with no proration in either direction"
says "A pass SHALL cost the full price **regardless of how many sessions
remain** when it is bought". At zero remaining sessions that sentence
reads as a mandate to charge. The requirement is corrected to keep flat
pricing while putting a floor under it.

## What Changes

- Add a private helper on `SeriesEnrollmentService` that returns
  `Err(AppError::BadRequest)` when the series has no occurrence whose
  `start_utc()` is in the future, and call it at the top of both public
  entry points — `enroll` and `enroll_guest`. Two call sites, one
  implementation, covering the member portal, the public endpoint, and any
  future caller.
- The check is deliberately **not** applied to the admin roster's
  at-the-door / comp path
  (`src/web/portal/admin/events/enrollments.rs::record_enrollment_payment`),
  for the same reason that path already bypasses the capacity guard: an
  admin recording a late cash payment has made that call in the room.
- Flat pricing is otherwise untouched: one remaining session out of six
  still costs the full price, and a mid-class refund is still full.

## Impact

- `src/service/series_enrollment_service.rs` — new remaining-sessions
  guard, called from `enroll` and `enroll_guest`.
- Spec delta: `openspec/specs/paid-events/spec.md` — the requirement
  "Pass pricing is flat, with no proration in either direction" is
  modified to state the floor.
- No change to `src/web/templates/class_register.rs` (`is_over` already
  hides the form) or to the portal's `!is_past` control gating; those stay
  as the page-level courtesy in front of the endpoint-level rule.
