# paid-events Specification

## ADDED Requirements

### Requirement: A recurring series carries pass pricing and must be bounded to be priced

A recurring series SHALL carry `member_price_cents` and `guest_price_cents`
columns declared `NOT NULL DEFAULT 0` and a `guest_registration_enabled` boolean
declared `NOT NULL DEFAULT false`, with the same semantics those columns already
carry on an event: `0` means free, a value above `0` is the amount paid, `NULL`
SHALL NOT be used to mean free, and existing series backfill to `0`.

A series whose pass price is greater than `0` SHALL have a non-null `until_date`.
Setting a pass price on an open-ended series SHALL be rejected with a
`BadRequest` explaining that a priced class must have an end date, because a flat
price buying unlimited future sessions is a subscription rather than a pass, and
subscriptions are served by the existing recurring-billing capability.

A series with a pass price greater than `0` is a **paid class**.

#### Scenario: A pass price on an open-ended series is rejected

- **WHEN** an admin sets a pass price on a series whose `until_date` is `NULL`
- **THEN** the call SHALL return `BadRequest` naming the missing end date, and no
  price SHALL be persisted

#### Scenario: A bounded series accepts a pass price

- **WHEN** an admin sets a pass price on a series with an `until_date` six weeks
  out
- **THEN** the price SHALL be persisted and the series SHALL be a paid class

#### Scenario: Existing series remain free

- **WHEN** the migration adding the pricing columns runs
- **THEN** every existing series SHALL have `member_price_cents = 0` and
  `guest_price_cents = 0` and SHALL remain free

### Requirement: Enrolling in a paid class takes payment before enrollment is confirmed

Enrollment in a paid class SHALL follow the ordering a single paid seat already
follows: claim the enrollment within a transaction as `PendingPayment`, then
create the `Pending` payment and the Stripe Checkout session, releasing the claim
if session creation fails.

The payment SHALL carry `PaymentKind::SeriesPass { series_id }`, and the
enrollment SHALL be confirmed only by the checkout-completion webhook, never by
the browser's return to the `success_url`. Enrollment SHALL be released by the
same payment-status transitions that release a single seat, so an abandoned
checkout frees the class seat without new logic.

A person SHALL NOT be enrolled twice in the same series: an existing `Completed`
pass SHALL charge nothing and return the existing enrollment, and an existing
`Pending` one SHALL return the in-flight Checkout session.

#### Scenario: Buying a pass reaches Stripe with the enrollment held

- **WHEN** a member enrolls in a paid class with seats available
- **THEN** a `series_enrollment` row SHALL exist as `PendingPayment` linked to a
  `Pending` `SeriesPass` payment, and the member SHALL be sent to Checkout

#### Scenario: Completing checkout confirms the enrollment

- **WHEN** the `series_pass` checkout-completion webhook arrives
- **THEN** the payment SHALL become `Completed` and the enrollment SHALL become
  confirmed

#### Scenario: Buying the same pass twice charges once

- **WHEN** someone with a `Completed` pass for a series submits enrollment again
- **THEN** no new payment and no new Checkout session SHALL be created

### Requirement: Enrollment materializes attendance for every future occurrence

Confirming an enrollment SHALL create an `event_attendance` row for every
occurrence of the series that has not yet started, so per-occurrence rosters,
check-in, reminders, and feeds continue to read one table with no additional
query paths.

The daily horizon roll-forward SHALL create attendance rows for newly
materialized occurrences of a series that has active enrollments, so an enrollee
does not silently vanish from sessions materialized after they enrolled.

Occurrences that have already started SHALL NOT be back-filled for a late
enrollee, because a roster asserting attendance at a session that already
happened is false and check-in data would inherit that falsehood.

#### Scenario: A confirmed pass puts the attendee on every future night

- **WHEN** a member's pass for a six-session class is confirmed before session one
- **THEN** attendance rows SHALL exist for all six occurrences

#### Scenario: A late enrollee is added only to remaining sessions

- **WHEN** someone enrolls after two of six sessions have already started
- **THEN** attendance rows SHALL exist for the four remaining occurrences and NOT
  for the two that already started

#### Scenario: Roll-forward extends active enrollments

- **WHEN** the horizon roll-forward materializes a new occurrence of a series with
  an active enrollment
- **THEN** an attendance row for that enrollee SHALL be created for the new
  occurrence

### Requirement: Capacity for a paid class is enforced once, at the series level

A paid class SHALL carry its own capacity, enforced when an enrollment is claimed,
counting confirmed enrollments plus in-flight ones whose payment is still
`Pending` — the same held-seat rule a single paid event uses.

Attendance rows created by a confirmed enrollment SHALL NOT be re-checked against
an individual occurrence's `max_attendees`. The enrollee already paid for the
class; rejecting them at session four would be money taken for a seat that does
not exist, which is the failure the paid-events capability exists to prevent.

The enrollment count and the enrollment insert SHALL happen in one transaction so
two people cannot claim the last place in a class concurrently.

#### Scenario: A full class rejects further enrollment

- **WHEN** someone enrolls in a paid class whose held enrollments equal its
  capacity
- **THEN** the call SHALL return `BadRequest` with no enrollment, payment, or
  Checkout session created

#### Scenario: A pass-holder is never bounced from a session

- **WHEN** an occurrence's `max_attendees` is lower than the number of confirmed
  pass-holders
- **THEN** every pass-holder SHALL still hold an attendance row for that
  occurrence

#### Scenario: Concurrent enrollment for the last place yields one winner

- **WHEN** two enrollments for the final place in a class are processed
  concurrently
- **THEN** exactly one SHALL claim it and the other SHALL be rejected

### Requirement: Pass pricing is flat, with no proration in either direction

A pass SHALL cost the full price regardless of how many sessions remain when it is
bought, and a refunded pass SHALL be refunded in full regardless of how many
sessions have already been held.

No per-session proration, partial refund, or make-up credit SHALL be computed.
This is a deliberate policy, recorded here so that a future contributor
implements the flat rule on purpose rather than treating its simplicity as an
oversight: per-session accounting introduces questions this capability declines to
answer (what a cancelled occurrence is worth, what a partially-attended class is
worth) that no organization has yet needed answered.

#### Scenario: A late enrollee pays full price

- **WHEN** someone enrolls in a six-session class with two sessions remaining
- **THEN** they SHALL be charged the full pass price

#### Scenario: A mid-class refund returns the full amount

- **WHEN** an admin refunds a pass after four of six sessions have been held
- **THEN** the full pass price SHALL be refunded, not a prorated remainder

### Requirement: Refunding a pass cancels the enrollment and its remaining seats

A refunded series-pass payment SHALL cancel the enrollment and SHALL cancel the
enrollee's attendance rows for occurrences that have not yet started, whether the
refund is issued through the admin refund route or observed via a `charge.refunded`
webhook.

Attendance rows for occurrences that already happened SHALL be retained, because
they are a record of who was present and SHALL NOT be rewritten by a later
financial event.

#### Scenario: Refund releases the remaining sessions

- **WHEN** a pass is refunded after two of six sessions
- **THEN** the enrollment SHALL be cancelled, the four future attendance rows
  SHALL be cancelled, and the class SHALL have a place free again

#### Scenario: Past attendance survives a refund

- **WHEN** a pass is refunded for someone who attended the first two sessions
- **THEN** the attendance records for those two sessions SHALL remain

### Requirement: Deleting a paid class refunds every enrollee before deletion

Deleting a series that has `Completed` series-pass payments SHALL refund every
such payment first and SHALL abort the deletion if any refund fails, returning the
error and leaving the series in place.

This mirrors the rule for deleting a single paid event and exists for the same
reason: occurrences and their attendance cascade on series delete, so a
delete-then-refund ordering would destroy the roster while the charges stood. A
class that cannot be fully refunded SHALL remain visible and fixable.

#### Scenario: Deleting a paid class refunds its enrollees

- **WHEN** an admin deletes a paid class with five confirmed enrollments
- **THEN** all five SHALL be refunded before the series is deleted

#### Scenario: A failed refund aborts the series delete

- **WHEN** one enrollee's refund fails during deletion of a paid class
- **THEN** the series SHALL NOT be deleted and the error SHALL be surfaced

### Requirement: Cancelling one occurrence of a paid class is not a refund event

Cancelling a single occurrence of a paid class SHALL NOT refund any enrollee and
SHALL NOT cancel any enrollment; it SHALL cancel only that occurrence's attendance
rows.

A skipped week is a normal part of running a class — a holiday, a snow day, an
instructor conflict — and under this capability's flat pricing it does not entitle
anyone to money back. An organization that wants to compensate for a lost session
does so by extending the series or issuing a refund deliberately, not by having
one automatically fire from an occurrence exception.

#### Scenario: A holiday skip refunds nobody

- **WHEN** an admin cancels one occurrence of a paid class
- **THEN** no refund SHALL be issued, every enrollment SHALL remain confirmed, and
  only that occurrence's attendance rows SHALL be cancelled

### Requirement: Admins manage a paid class at the series level

The admin series detail page SHALL show an enrollment roster listing each enrollee
with their identity and payment state, and SHALL offer the same actions a
single-event roster offers — recording an at-the-door payment, comping an
enrollment, releasing a stuck `PendingPayment` enrollment, and refunding — applied
at series scope.

Every one of those actions SHALL be admin-authenticated, CSRF-protected, and
audited on the same terms as their single-event equivalents.

Per-occurrence rosters SHALL continue to work unchanged and SHALL show
pass-holders alongside any single-event registrants.

#### Scenario: The series roster shows who bought the class

- **WHEN** an admin opens a paid class's detail page
- **THEN** every enrollee SHALL be listed with their payment state

#### Scenario: Comping an enrollment enrolls without a charge

- **WHEN** an admin comps someone into a paid class
- **THEN** the enrollment SHALL be confirmed, a `Waived` series-pass payment SHALL
  be recorded, attendance rows SHALL be created for future occurrences, and no
  Stripe charge SHALL occur

#### Scenario: The per-occurrence roster still works

- **WHEN** an admin opens one occurrence of a paid class
- **THEN** the pass-holders SHALL appear on that occurrence's roster
