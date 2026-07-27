# paid-events Specification

## ADDED Requirements

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
