# paid-events Specification

## ADDED Requirements

### Requirement: An event carries a member price where zero means free

An event SHALL carry a `member_price_cents` column declared `NOT NULL DEFAULT 0`,
in which `0` SHALL mean the event costs a member nothing and any value greater
than `0` SHALL be the amount a member pays. An event is **paid for members**
exactly when `member_price_cents > 0`.

`NULL` SHALL NOT be used to represent a free event. `NULL` conventionally means
"unknown" or "not entered", and using it as a sentinel for zero makes ordinary
queries wrong in ways that fail silently rather than loudly: `WHERE
member_price_cents = 0` would match no free event at all, `WHERE
member_price_cents <= 2000` would omit them (a SQL comparison against `NULL` is
unknown, not true), and `SUM`, `AVG`, and `ORDER BY` would each skip them. A
price of zero is a *known* price, so it SHALL be stored as the number zero and a
caller SHALL NOT have to know a `NULL` convention to find free events.

An admin who leaves the price field blank SHALL have `0` stored, and an admin who
types `0` SHALL have `0` stored; both express the same intent and neither SHALL
produce a validation error. A NEGATIVE price SHALL be rejected with a
`BadRequest`, because unlike `0` it expresses no coherent intent. A price greater
than `MAX_PAYMENT_CENTS` SHALL likewise be rejected.

The migration SHALL backfill existing events to `0`, which is not a guess but a
true statement about every event predating this feature.

The price SHALL be settable on the existing admin event create and update forms.
Changing the price SHALL NOT retroactively alter what an already-registered
attendee paid.

#### Scenario: A blank price field stores zero, not null

- **WHEN** an admin creates an event and leaves the price field empty
- **THEN** `member_price_cents` SHALL be persisted as `0`, never `NULL`, and a
  member registering for it SHALL be registered immediately without any payment
  step

#### Scenario: A typed zero is stored as a real zero

- **WHEN** an admin submits an event form with a price of `0`
- **THEN** the form SHALL succeed and `member_price_cents` SHALL be persisted as
  `0`; no validation error SHALL be shown and no rewriting to `NULL` SHALL occur

#### Scenario: Free events are findable by the obvious query

- **WHEN** a caller looks for events that cost members nothing using
  `WHERE member_price_cents = 0`
- **THEN** every free event SHALL be returned, and a range query such as
  `WHERE member_price_cents <= 2000` SHALL likewise include them

#### Scenario: A negative price is rejected

- **WHEN** an admin submits an event form with a price below `0`
- **THEN** the handler SHALL return `BadRequest` and SHALL NOT persist a price

#### Scenario: Pre-existing events are backfilled to zero

- **WHEN** the migration adding the column runs against a database whose events
  predate paid events
- **THEN** every existing row SHALL have `member_price_cents = 0` and SHALL remain
  free to members

#### Scenario: Raising the price does not re-bill existing attendees

- **WHEN** an admin raises the price of an event that already has paid attendees
- **THEN** the existing attendees SHALL remain registered, SHALL NOT be charged
  the difference, and their recorded payment amounts SHALL be unchanged

### Requirement: Registering for a paid event holds a seat and routes through Stripe Checkout

Registration for a paid event SHALL claim the seat BEFORE creating the Stripe
Checkout session, in this order: (1) within a single transaction, count the held
seats and insert an `event_attendance` row with status `PendingPayment`; (2)
create the `Pending` payment row and the Checkout session; (3) if step 2 fails,
release the claimed seat and return the error.

The Checkout session SHALL be stamped with session metadata `payment_type =
"event_fee"` and the event id, so the webhook dispatcher can branch on it exactly
as it already branches on `membership` and `donation`. The payment SHALL be
recorded with `PaymentKind::EventFee { event_id }` and the attendance row SHALL
store the resulting `payment_id`.

The reverse order — creating the session before claiming the seat — SHALL NOT be
used: it admits a window in which two members both pay for the last seat, and
turns a rejected registration into a refund incident.

#### Scenario: Paid registration returns a Checkout redirect, not an immediate seat

- **WHEN** an Active member registers for an event with `member_price_cents` set
- **THEN** an `event_attendance` row SHALL exist with status `PendingPayment`
  linked to a `Pending` `EventFee` payment, and the member SHALL be sent to the
  Stripe Checkout URL; the member SHALL NOT yet count as `Registered`

#### Scenario: Session-creation failure releases the claimed seat

- **WHEN** the seat has been claimed as `PendingPayment` and the Stripe Checkout
  session creation then fails
- **THEN** the claimed seat SHALL be released so it does not remain held by a
  registration that can never be paid, and the caller SHALL receive an error

#### Scenario: Free-event registration is unchanged

- **WHEN** a member registers for an event whose `member_price_cents` is `0`
- **THEN** no payment row and no Checkout session SHALL be created and the
  attendance row SHALL go straight to `Registered`

### Requirement: Paid-event capacity is enforced against confirmed and in-flight seats

A paid event with a non-null `max_attendees` SHALL reject a registration that
would exceed it, returning `BadRequest`, and the seat count SHALL include both
confirmed seats and in-flight ones. A seat SHALL be counted as held when the
attendance row is `Registered`, OR when it is `PendingPayment` AND its linked
payment is still `Pending`.

A `PendingPayment` row whose linked payment is no longer `Pending` SHALL NOT hold
a seat, so an abandoned checkout frees its seat by virtue of its payment being
flipped, without requiring the row to be deleted.

The count and the insert SHALL happen in the same transaction so two concurrent
registrations for the last seat cannot both succeed.

Free events SHALL retain their existing advisory capacity behavior; this
requirement applies only to events with a price.

#### Scenario: Registration for a sold-out paid event is rejected

- **WHEN** a member registers for a paid event whose held seats already equal
  `max_attendees`
- **THEN** the call SHALL return `BadRequest`, no `event_attendance` row SHALL be
  inserted, no payment row SHALL be created, and no Checkout session SHALL be
  minted

#### Scenario: An abandoned checkout's seat becomes available again

- **WHEN** a paid event is at capacity solely because of a `PendingPayment` seat
  whose payment has since been flipped to `Failed`
- **THEN** that seat SHALL NOT be counted as held and a new registration SHALL
  succeed

#### Scenario: Two members racing for the last seat produce exactly one winner

- **WHEN** two registrations for the final seat of a paid event are processed
  concurrently
- **THEN** exactly one SHALL claim the seat and reach Checkout; the other SHALL
  receive `BadRequest` without a payment row or a Checkout session

### Requirement: Completing checkout confirms the seat

The webhook dispatcher SHALL, on a `checkout.session.completed` event whose
session metadata carries `payment_type = "event_fee"`, flip the payment row to
`Completed` and flip the linked `event_attendance` row from `PendingPayment` to
`Registered`. The seat SHALL be confirmed by the webhook and never by the
browser's return to the Checkout `success_url`, which carries no trust.

Confirmation SHALL be idempotent under Stripe's at-least-once delivery: a
redelivered event SHALL leave the payment `Completed` and the seat `Registered`
without duplicating either the row or any side effect. Event-fee completion SHALL
NOT extend dues and SHALL NOT touch the member's auto-renew schedule.

Confirming a seat SHALL emit the audit-log row required by "Paid-registration
money transitions are audited" below, and SHALL emit it exactly once — on the
delivery that actually flips the payment, not on a redelivery that finds it
already `Completed`.

#### Scenario: Paid checkout turns the held seat into a confirmed one

- **WHEN** a `checkout.session.completed` event arrives for an `event_fee`
  session
- **THEN** the payment SHALL become `Completed` and the attendance row SHALL
  become `Registered`

#### Scenario: Redelivery of the completion event changes nothing further

- **WHEN** Stripe redelivers the same `event_fee` completion event
- **THEN** the payment SHALL remain `Completed`, the seat SHALL remain
  `Registered`, and no second payment row or duplicate side effect SHALL result

#### Scenario: An event-fee payment does not extend dues

- **WHEN** an `event_fee` checkout completes for a member
- **THEN** the member's `dues_paid_until` SHALL be unchanged and no auto-renew
  reschedule SHALL be performed

### Requirement: An abandoned or failed checkout releases the seat

An event-fee payment that is flipped to `Failed` SHALL release its seat, and the
existing `checkout.session.expired` and `payment_intent.payment_failed` handlers
SHALL be sufficient to achieve this without new release logic, because the
held-seat count already excludes a `PendingPayment` row whose payment is not
`Pending`.

The `PendingPayment` attendance row SHALL be retained rather than deleted so the
admin roster can distinguish "started paying and did not finish" from "never
registered". An event-fee Checkout session SHALL be created with an expiry of no
more than 60 minutes so an abandoned seat is bounded.

#### Scenario: Expired checkout frees the seat

- **WHEN** a `checkout.session.expired` event arrives for an `event_fee` session
- **THEN** the payment SHALL become `Failed` and the seat SHALL no longer be
  counted against capacity

#### Scenario: The abandoned attempt stays visible to admins

- **WHEN** an admin views the roster after a member abandoned a paid checkout
- **THEN** the abandoned attempt SHALL still be listed, distinguishable from a
  confirmed registration

### Requirement: A member is not charged twice for the same event

Registration SHALL be idempotent per member per event: before claiming a seat the
service SHALL look for an existing non-`Failed` event-fee payment by this member
for this event. When a `Completed` one exists it SHALL return the existing
confirmed registration and charge nothing; when a `Pending` one exists it SHALL
return that in-flight session rather than minting a second Checkout session.

This exists because double-clicking the register button and using the browser
back button are the realistic ways a member gets billed twice.

#### Scenario: Registering again after paying charges nothing

- **WHEN** a member who already has a `Completed` event-fee payment for an event
  submits the registration again
- **THEN** no new payment row and no new Checkout session SHALL be created and
  the member SHALL remain `Registered`

#### Scenario: Double-submitting reuses the in-flight checkout

- **WHEN** a member with a `Pending` event-fee payment for an event submits the
  registration again
- **THEN** the existing Checkout session SHALL be returned and a second seat
  SHALL NOT be claimed

### Requirement: Refunding an event-fee payment releases the seat

A refunded event-fee payment SHALL release its seat by flipping the linked
`event_attendance` row to `Cancelled`, whether the refund is issued through the
admin refund route or observed out-of-band via a `charge.refunded` webhook from
the Stripe dashboard.

A member SHALL NOT retain a confirmed seat for an event whose fee has been
returned to them.

#### Scenario: Admin refund cancels the registration

- **WHEN** an admin refunds a `Completed` event-fee payment
- **THEN** the payment SHALL become `Refunded`, the attendance row SHALL become
  `Cancelled`, and the seat SHALL be available to another member

#### Scenario: Out-of-band Stripe refund also releases the seat

- **WHEN** a `charge.refunded` event arrives for an event-fee payment refunded
  from the Stripe dashboard
- **THEN** the linked attendance row SHALL become `Cancelled`

### Requirement: Deleting a paid event refunds every paid attendee before the event is removed

Deleting an event that has `Completed` event-fee payments SHALL refund every such
payment first and SHALL abort the deletion if any refund fails, returning the
error and leaving the event in place.

This ordering is required because `event_attendance` cascades on event delete: a
delete-then-refund ordering would destroy the roster while the charges stood,
leaving unrefundable money and no record of who was owed it. An event that cannot
be fully refunded SHALL remain visible and fixable rather than becoming an
invisible pile of unreturned charges.

#### Scenario: Deleting a paid event refunds its attendees

- **WHEN** an admin deletes an event with three `Completed` event-fee payments
- **THEN** all three SHALL be refunded and only then SHALL the event be deleted

#### Scenario: A failed refund aborts the delete

- **WHEN** one attendee's refund fails while deleting a paid event
- **THEN** the event SHALL NOT be deleted, the error SHALL be surfaced to the
  admin, and the remaining roster SHALL be intact

### Requirement: Paid-registration money transitions are audited

Every transition that moves money or changes a paid seat's state SHALL emit an
audit-log row via `audit_service.log`. The audited transitions SHALL be: a seat
confirmed by checkout completion, an event-fee payment refunded (by either the
admin route or an out-of-band `charge.refunded`), an at-the-door payment
recorded, a seat comped, a stuck `PendingPayment` seat released, and the
bulk refund performed when a paid event is deleted.

For the webhook-driven transitions the audit row's actor SHALL identify the
system/Stripe source rather than a member, consistent with the existing rule that
webhook-recorded and admin-recorded payments describe the same business action
differing only in actor and source fields.

Free-event RSVP transitions SHALL remain unaudited — this requirement covers only
paid seats, and does not change today's free-RSVP behavior.

Claiming a seat as `PendingPayment` SHALL NOT be audited on its own: no money has
moved yet, and auditing every abandoned checkout would bury the rows that matter.

#### Scenario: A confirmed paid seat writes an audit row

- **WHEN** an `event_fee` checkout completes and the seat becomes `Registered`
- **THEN** an audit-log row SHALL be written identifying the member, the event,
  and the payment

#### Scenario: Redelivery does not write a second audit row

- **WHEN** Stripe redelivers a completion event for an already-`Completed`
  event-fee payment
- **THEN** no additional audit-log row SHALL be written

#### Scenario: Abandoning a checkout writes no audit row

- **WHEN** a member claims a seat as `PendingPayment` and never pays
- **THEN** no audit-log row SHALL be written for the claim or its expiry

#### Scenario: Free RSVP remains unaudited

- **WHEN** a member RSVPs to a free event
- **THEN** no audit-log row SHALL be written, unchanged from today's behavior

### Requirement: Admins can manage a paid event's roster

The admin event detail page SHALL show, for a paid event, a roster listing each
attendee with their registration status and payment state (paid, awaiting
payment, abandoned, refunded, comped, or paid at the door).

The roster SHALL offer: recording an at-the-door payment (via
`PaymentService::record_manual` with `PaymentMethod::Manual`), comping a seat
(via `record_manual` with `PaymentMethod::Waived`), and releasing a seat that is
stuck in `PendingPayment`. Releasing a stuck seat SHALL NOT issue a refund — it
exists for the case where a webhook never arrived, and an operator who needs the
money returned uses the refund route.

All three actions SHALL be admin-authenticated, CSRF-protected, and audited.

#### Scenario: Roster distinguishes paid from awaiting-payment

- **WHEN** an admin views the roster of a paid event with one confirmed attendee
  and one member currently at Stripe
- **THEN** the first SHALL show as paid and the second as awaiting payment

#### Scenario: Comping a seat registers the member without a charge

- **WHEN** an admin comps a member onto a paid event
- **THEN** the member SHALL become `Registered`, a `Waived` event-fee payment
  SHALL be recorded, and no Stripe charge SHALL occur

#### Scenario: Releasing a stuck seat does not refund

- **WHEN** an admin releases a seat stuck in `PendingPayment`
- **THEN** the seat SHALL stop being held and no refund SHALL be issued
