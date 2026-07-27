# paid-events Specification

## Purpose
TBD - created by archiving change a41-paid-events-member-registration. Update Purpose after archive.
## Requirements
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

### Requirement: Guest pricing and guest-registration eligibility are separate fields

An event SHALL carry a `guest_price_cents` column declared `NOT NULL DEFAULT 0`
holding what a non-member pays, and a separate `guest_registration_enabled`
boolean column declared `NOT NULL DEFAULT false` recording whether non-members may
register at all. Each column SHALL answer exactly one question: the price answers
"how much", the flag answers "whether".

These SHALL NOT be collapsed into a single nullable price where an absent value
means "no public registration". That encoding would make one column answer two
unrelated questions, so "the public attends free" and "the public may not attend"
would be indistinguishable — and it would reintroduce the `NULL`-as-zero problem
the member price already rejects, where `WHERE guest_price_cents = 0` matches no
free event and range filters silently omit them.

`guest_price_cents` SHALL follow the member price's rules exactly: a blank field
and a typed `0` both store `0`, negatives and values above `MAX_PAYMENT_CENTS` are
rejected, and existing events backfill to `0`.

Because the two fields are independent, all of these SHALL be expressible and all
SHALL be served: free for members and paid for the public; paid for both at
different amounts; paid for members with no public registration at all; and public
registration at no charge — the free-workshop-with-limited-seats case.

#### Scenario: Free for members, paid for the public

- **WHEN** an admin enables guest registration, sets a guest price, and leaves the
  member price blank
- **THEN** a member SHALL register with no payment step and a guest SHALL be
  required to pay the guest price

#### Scenario: Disabled guest registration is distinguishable from a zero guest price

- **WHEN** one event has `guest_registration_enabled = false` and another has
  `guest_registration_enabled = true` with `guest_price_cents = 0`
- **THEN** the two SHALL be distinguishable in storage and in queries; the first
  SHALL mean non-members may not register and the second SHALL mean non-members
  would attend at no charge

#### Scenario: A zero guest price is stored as zero

- **WHEN** an admin submits an event form with a guest price of `0`
- **THEN** `guest_price_cents` SHALL be persisted as `0`, never `NULL`, and no
  validation error SHALL be shown

### Requirement: Whether the public may register is independent of what it costs

The public registration page and endpoint SHALL serve an event when `visibility`
is `Public` AND `guest_registration_enabled` is true, and SHALL respond `404 Not
Found` otherwise. The guest price SHALL NOT be part of this test.

Whether an event requires registration, whether non-members may register, and what
attendance costs are three independent questions, and the data model SHALL keep
them independent:

- `rsvp_required` — whether attendees must register at all. This field already
  exists and SHALL remain the answer to that question; no parallel flag SHALL be
  introduced for it.
- `guest_registration_enabled` — whether non-members may register.
- `guest_price_cents` — what a non-member pays, where `0` means free.

Conflating price with eligibility would make a free-but-limited-capacity event —
a weekend workshop with twenty seats and no fee — unrepresentable, even though it
is a common and legitimate offering. An organization SHALL be able to require
registration without charging for it.

A `MembersOnly` or `AdminOnly` event SHALL NOT become readable or registerable
through the public registration URL. The response SHALL be `404`, NOT `403`,
because a `403` on a members-only event id confirms that the event exists and
leaks the existence of non-public events to anyone enumerating ids.

#### Scenario: A free workshop is publicly registerable

- **WHEN** an event is `Public`, has guest registration enabled, and has a guest
  price of `0`
- **THEN** the public registration page SHALL be served and a guest SHALL be able
  to register without any payment step

#### Scenario: An ordinary free event without guest registration is not registerable

- **WHEN** an event is `Public` with a guest price of `0` and
  `guest_registration_enabled` false — the common recurring-talk case
- **THEN** the public registration page SHALL return `404`; a zero price SHALL NOT
  by itself open a public door

#### Scenario: A members-only event 404s on the public page

- **WHEN** an anonymous visitor requests the public registration page for a
  `MembersOnly` event
- **THEN** the response SHALL be `404 Not Found` and SHALL disclose nothing about
  the event

#### Scenario: Enumeration cannot distinguish "private" from "absent"

- **WHEN** an anonymous visitor requests the public registration page for a
  `MembersOnly` event id and for a nonexistent event id
- **THEN** the two responses SHALL be indistinguishable

### Requirement: Coterie hosts a shareable public registration page

The system SHALL serve a public registration page at `GET /events/:id/register`
for every publicly registerable event, so an organizer can share one URL without
per-event work on any external site.

The page SHALL render only fields that are already public for a `Public` event:
title, description, start time in the event's timezone, location, the guest
price, and whether seats remain. It SHALL NOT render the attendee roster, which
is not public information, and SHALL NOT render members-only fields.

The page SHALL state the guest price, rendering a price of `0` as free rather
than as a currency amount of zero.

When the member price differs from the guest price, the page SHALL display the
member price alongside the guest price together with a link to log in, so a
member can choose member pricing instead of unknowingly paying the guest price.
This SHALL include the case where members attend free (`member_price_cents = 0`),
which is precisely the case where an unknowing member overpays the most.

When the event has no seats remaining the page SHALL say so and SHALL NOT offer a
registration form.

#### Scenario: The public page shows the event and its guest price

- **WHEN** an anonymous visitor opens the registration page for a publicly
  registerable event
- **THEN** the page SHALL show the event's public details and the guest price, and
  SHALL NOT show the roster

#### Scenario: A member is offered member pricing rather than silently overcharged

- **WHEN** the event's member price differs from its guest price and an anonymous
  visitor opens the public page
- **THEN** the page SHALL display the member price and a login link alongside the
  guest price

#### Scenario: A sold-out event offers no registration form

- **WHEN** the event's held seats equal `max_attendees`
- **THEN** the page SHALL indicate that it is full and SHALL NOT present a
  registration form

### Requirement: Guest registration takes payment before the seat is confirmed

Guest registration for an event with a guest price greater than `0` SHALL follow
the same ordering the member path follows: claim the seat as `PendingPayment`
within a transaction, then create the `Pending` event-fee payment and the Stripe
Checkout session, releasing the claimed seat if session creation fails.

A paid guest seat SHALL be confirmed only by the checkout-completion webhook,
SHALL count toward capacity on exactly the same terms as a member seat, and SHALL
be released by the same payment-status transitions. No separate guest lifecycle
SHALL be introduced.

When the guest price is `0` the registration SHALL claim the seat and confirm it
immediately as `Registered`, creating no payment row and no Checkout session —
mirroring how a member registers for a free event. Capacity SHALL be enforced on
the same terms.

Free public registration SHALL rely on the bot challenge and the per-IP rate limit
as its only abuse controls, since no card provides friction. This is a known
weaker position than the paid path: a determined abuser can consume seats with
fabricated email addresses. It is accepted because the alternative — refusing to
serve free registration at all — makes the free-workshop case unrepresentable, and
because an admin can release seats from the roster. An organization running a
high-demand free event SHOULD be aware that the seats are only as trustworthy as
the challenge in front of them.

The guest's supplied name and email SHALL be recorded on the attendance row, and
for a paid registration carried onto the payment as a non-member payer so the
payment has an identity for receipts.

#### Scenario: A guest reaches Stripe with a held seat

- **WHEN** a guest submits the public registration form for an available event
- **THEN** an attendance row SHALL exist with status `PendingPayment` and guest
  identity, linked to a `Pending` event-fee payment, and the guest SHALL be sent
  to Stripe Checkout

#### Scenario: A guest seat counts against capacity like a member seat

- **WHEN** a paid event's remaining capacity is one and a guest holds it as
  `PendingPayment` with a `Pending` payment
- **THEN** a subsequent member or guest registration SHALL be rejected as full

#### Scenario: An abandoned guest checkout frees the seat

- **WHEN** a guest's checkout session expires without payment
- **THEN** the payment SHALL become `Failed` and the seat SHALL stop being held,
  by the same rule that governs member seats

#### Scenario: Registering for a free workshop skips checkout entirely

- **WHEN** a guest registers for an event whose guest price is `0`
- **THEN** the seat SHALL be confirmed as `Registered` immediately, no payment row
  and no Checkout session SHALL be created, and the seat SHALL count against
  capacity

#### Scenario: A free registration still passes the bot challenge and rate limit

- **WHEN** a guest submits a free registration and the bot-challenge token is
  missing or the IP is over the rate limit
- **THEN** the request SHALL be rejected and no seat SHALL be claimed; the absence
  of a payment step SHALL NOT relax either control

### Requirement: The public registration endpoint is protected as a public money endpoint

`POST /public/events/:id/register` SHALL be protected by the same layers that
protect the existing public money endpoints, applied in this order: CORS
allowlist, then the per-IP `money_limiter`, then bot-challenge verification, then
the handler.

The rate limiter SHALL run BEFORE the bot-challenge provider so a bursting client
cannot burn the organization's provider quota — the ordering already required for
`/public/signup`. Bot-challenge verification SHALL fail closed exactly as it does
for the other public endpoints.

This endpoint initiates a Stripe charge from an unauthenticated caller, which is
the same exposure that produced card-testing abuse against `/public/signup` and
`/public/donate`; shipping it without these layers SHALL be treated as a defect
rather than a follow-up.

#### Scenario: A registration flood is rate-limited before the provider is called

- **WHEN** an IP at the `money_limiter` budget submits another public registration
- **THEN** the request SHALL be rejected with `429` WITHOUT consulting the
  bot-challenge provider

#### Scenario: A missing bot-challenge token fails closed

- **WHEN** the bot-challenge provider is configured and a public registration
  request omits the token
- **THEN** the request SHALL be rejected with `403` and no payment row, seat, or
  Checkout session SHALL be created

### Requirement: A guest registration is never attached to a member account by email

Guest registration SHALL NOT look up the supplied email against the member
directory and SHALL NOT attach the resulting seat or payment to a matching member
account; a guest registration stays a guest registration even when the email
matches a member exactly.

This deliberately differs from `/public/donate`, which does attach a matching
donation to the member. The difference is justified: a donation is a gift whose
attribution is helpful and harmless, whereas a registration is a priced seat, and
the email on this endpoint is unverified input from an unauthenticated caller.
Auto-matching would let an anonymous caller write an attendance row and a payment
row into a named member's account and select that member's price bracket, purely
by typing their address.

A member who wants member pricing SHALL obtain it by logging in and registering
through the portal, which the public page links to.

#### Scenario: A guest using a member's email still gets a guest seat

- **WHEN** a guest registers with an email address that exactly matches an
  existing member
- **THEN** the attendance row SHALL be a guest row, the payment SHALL record a
  non-member payer, and no row SHALL be written into the matching member's account

### Requirement: A guest is not charged twice nor seated twice for the same event

Guest registration SHALL be idempotent per event per guest email on the same
terms the member path is idempotent per event per member. For a paid event: an
existing `Completed` event-fee payment for that event and email SHALL charge
nothing and return the existing registration, and an existing `Pending` one SHALL
return the in-flight Checkout session rather than minting a second. For a free
event: an existing confirmed seat for that email SHALL be returned unchanged
rather than producing a second seat.

The database SHALL enforce one seat per guest email per event so a concurrent
double submission cannot produce two seats. That constraint SHALL be the
guarantee for both the paid and free paths — the free path has no payment row to
deduplicate against, so the uniqueness of the seat itself is what prevents a
double booking.

#### Scenario: Re-submitting after paying charges nothing

- **WHEN** a guest who already has a `Completed` event-fee payment for an event
  submits the public form again with the same email
- **THEN** no new payment row and no new Checkout session SHALL be created

#### Scenario: Re-submitting a free registration seats nobody twice

- **WHEN** a guest already registered for a free event submits the public form
  again with the same email
- **THEN** their existing seat SHALL be returned and no second seat SHALL be
  created

#### Scenario: Concurrent double submission yields one seat

- **WHEN** the same guest email is submitted twice concurrently for one event
- **THEN** exactly one attendance row SHALL exist for that email and event

### Requirement: A confirmed guest receives an emailed confirmation

A guest whose seat is confirmed SHALL be sent an email containing the event
details, when an email provider is configured. For a paid registration that email
SHALL also carry a receipt for the amount paid; for a free registration it SHALL
confirm the seat with no receipt, because no money changed hands and a
zero-amount receipt is noise.

When no provider is configured the send SHALL be skipped silently and the
registration SHALL still stand — the same rule the existing receipt email
follows.

That email is the guest's only artifact of the registration, since a guest has no
portal account.

#### Scenario: A paid guest gets a confirmation email with a receipt

- **WHEN** a guest's event-fee checkout completes and an email provider is
  configured
- **THEN** an email SHALL be sent to the guest's address with the event details
  and the amount paid

#### Scenario: A free registrant gets a confirmation without a receipt

- **WHEN** a guest registers for a free event and an email provider is configured
- **THEN** an email SHALL be sent confirming the seat and the event details, and
  it SHALL NOT contain a payment receipt

#### Scenario: No email provider does not fail the registration

- **WHEN** a guest's registration is confirmed and no email provider is configured
- **THEN** no email SHALL be attempted and the guest's seat SHALL still be
  confirmed

### Requirement: Guests are identified as guests on the roster and in reporting

The admin roster SHALL show a guest attendee's supplied name and email and SHALL
visually distinguish guest registrations from member registrations, so an
organizer checking people in can tell who has a member account and who does not.

Admin actions defined for paid seats — recording an at-the-door payment, comping,
releasing a stuck `PendingPayment` seat, and refunding — SHALL be available for
guest rows on the same terms as member rows, and SHALL be audited identically.

#### Scenario: The roster distinguishes guests from members

- **WHEN** an admin views the roster of an event with one member attendee and one
  guest attendee
- **THEN** both SHALL be listed, the guest SHALL show the supplied name and email,
  and the two SHALL be visually distinguishable

#### Scenario: A guest seat can be refunded and released like a member seat

- **WHEN** an admin refunds a guest's `Completed` event-fee payment
- **THEN** the payment SHALL become `Refunded`, the guest attendance row SHALL
  become `Cancelled`, and the seat SHALL be available again

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

