# paid-events Specification Delta

## MODIFIED Requirements

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

That login link SHALL carry a return path back to the registration page the
visitor is standing on, so logging in returns them to the event they were trying
to register for rather than to the dashboard.

The login flow's post-authentication destination SHALL remain an allow-list of
site-relative paths, admitting the shareable registration paths in addition to
the portal paths it already admits, and SHALL continue to reject anything outside
that list — including absolute URLs and any path containing `..` — by falling
back to its default destination. Widening the destination to arbitrary
caller-supplied values would turn the most public page in the application into an
open redirect, which is a worse defect than the one this return path fixes.

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

#### Scenario: The login link returns the visitor to the event

- **WHEN** an anonymous visitor follows the page's login link and authenticates
- **THEN** they SHALL be returned to the registration page for that same event

#### Scenario: An off-site return path is refused

- **WHEN** a login request carries a post-authentication destination that is not
  in the allow-list — an absolute URL, or a path containing `..`
- **THEN** the destination SHALL be discarded and the default destination used

#### Scenario: A sold-out event offers no registration form

- **WHEN** the event's held seats equal `max_attendees`
- **THEN** the page SHALL indicate that it is full and SHALL NOT present a
  registration form

## ADDED Requirements

### Requirement: A shareable registration page recognizes a signed-in member

A shareable registration page SHALL resolve the request's member session before
rendering, and SHALL NOT treat a request carrying a valid member session as
anonymous. This governs both shareable pages: `GET /events/:id/register` and
`GET /classes/:id/register`.

When the session resolves to an authenticated member, the page SHALL offer that
member the authenticated registration path — the same path `/portal/events` uses,
priced at `member_price_cents` for an event and at the series member pass price
for a class — and SHALL NOT present the guest registration form. Offering the
guest form to a signed-in member is what causes a member to be charged the guest
price and seated as a guest, which can only be undone by a refund or by an admin
releasing and re-seating them.

Because the identity is established by the session, the page SHALL NOT ask a
signed-in member for a name or email, and SHALL NOT render a bot challenge: the
authenticated path is not an anonymous money endpoint and does not carry the
anonymous endpoint's protections or its need for them.

When the signed-in member already holds a seat for the event, or is already
enrolled in the class, the page SHALL say so and SHALL NOT present an action that
appears to charge them again.

Session resolution SHALL fail open to the anonymous rendering: an absent,
malformed, or expired session, or an error while resolving one, SHALL render the
guest page exactly as it renders today. A member seeing the guest form is the
bug this requirement fixes; a guest seeing an error page instead of a working
registration form would be a worse one.

The event's registerability rule is unchanged by the presence of a session: a
page that is not publicly registerable SHALL still respond `404` to a signed-in
member, so a session cannot be used to enumerate non-public events.

#### Scenario: A signed-in member is offered the member price, not the guest form

- **WHEN** a member with a valid session opens the shareable registration page for
  a paid event whose member price differs from its guest price
- **THEN** the page SHALL present the member-priced authenticated registration
  action and SHALL NOT present the guest name-and-email form

#### Scenario: A signed-in member is not asked for details already on file

- **WHEN** a member with a valid session opens the shareable registration page
- **THEN** the page SHALL NOT render name, email, or bot-challenge fields

#### Scenario: A signed-in member who already has a seat is told so

- **WHEN** a member with a valid session opens the registration page for an event
  they already hold a seat for
- **THEN** the page SHALL indicate they are already registered and SHALL NOT
  present a registration action

#### Scenario: A class page recognizes the session the same way

- **WHEN** a member with a valid session opens `GET /classes/:id/register` for a
  paid class
- **THEN** the page SHALL present the member pass price and the authenticated
  enrollment action rather than the guest enrollment form

#### Scenario: An expired session renders the guest page

- **WHEN** a request carries a session cookie that no longer resolves to a member
- **THEN** the page SHALL render exactly as it does for an anonymous visitor,
  including the guest form and the bot challenge

#### Scenario: A session does not make a non-registerable event visible

- **WHEN** a member with a valid session opens the registration page for an event
  that is not publicly registerable
- **THEN** the response SHALL be `404`
