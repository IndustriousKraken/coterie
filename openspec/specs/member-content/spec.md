# member-content Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Members see all events and announcements (public + members-only)

The member portal SHALL provide:
- `GET /portal/events` — events page.
- `GET /portal/announcements` — announcements page.
- `GET /portal/api/events/list` — HTMX events list fragment.
- `GET /portal/api/announcements/list` — HTMX announcements list fragment.
- `GET /portal/api/events/upcoming` — HTMX dashboard upcoming-events fragment.

Members SHALL see both public and members-only content; the public/private flag affects only the `/public/*` surface.

Events whose visibility is `AdminOnly` SHALL NOT appear on any member surface for a member who is not an admin — not in the events list fragment, not in the dashboard upcoming-events fragment, and not on the events page. `AdminOnly` is the level an organization uses for events the membership is not meant to know about, so its content — title, description, location, image, price — SHALL NOT be rendered to a non-admin. A member who IS an admin SHALL continue to see `AdminOnly` events on the member surfaces as well as in the admin surface, since the same content is already theirs to read.

The visibility decision SHALL have one home in the domain layer, so the rule is not re-derived per call site.

#### Scenario: Members-only event is visible inside the portal

- **WHEN** an authenticated member views the events page
- **THEN** members-only events SHALL appear alongside public ones

#### Scenario: An admin-only event is hidden from a non-admin member

- **WHEN** a non-admin Active member loads `GET /portal/api/events/list` or `GET /portal/api/events/upcoming` and an `AdminOnly` event is upcoming
- **THEN** that event SHALL be absent from the fragment, and none of its title, description, or location SHALL appear in the response

#### Scenario: An admin still sees admin-only events

- **WHEN** a member with `is_admin` loads the same member-portal fragments
- **THEN** `AdminOnly` events SHALL be listed alongside public and members-only ones

### Requirement: Members can RSVP to events

`POST /portal/api/events/:id/rsvp` and `POST /portal/api/events/:id/cancel` SHALL allow Active/Honorary members to manage their RSVP. For a **free** event (`member_price_cents = 0`) the handlers SHALL call `event_repo.register_attendance` / `cancel_attendance` and return an updated HTMX button fragment, exactly as before.

For a **paid** event (`member_price_cents > 0`) the RSVP handler SHALL NOT register the member directly. It SHALL instead route through the paid-registration path — claiming the seat as `PendingPayment` and returning a redirect to Stripe Checkout — per the `paid-events` capability. The member becomes `Registered` only when the checkout-completion webhook confirms payment.

The button label SHALL reflect which path applies, so a member can tell before clicking whether the control registers them or sends them to pay.

Both handlers SHALL refuse an event the requesting member may not see — an `AdminOnly` event requested by a non-admin — and SHALL answer with the same "event not found" response an unknown id produces. A distinguishable rejection would let a member confirm which admin-only event ids exist, which is the disclosure the level exists to prevent.

#### Scenario: RSVP is CSRF-protected

- **WHEN** an HTMX RSVP request arrives without `X-CSRF-Token`
- **THEN** the top-level CSRF layer SHALL reject it with 403

#### Scenario: RSVP changes are NOT currently audited

- **WHEN** a member RSVPs to, or cancels their RSVP for, a **free** event
- **THEN** no `audit_logs` row SHALL be written today; this is observed behavior. (Whether to audit free-event RSVP transitions is a policy question for a follow-up change; today's spec captures truth.) Paid registrations are the exception and SHALL be audited, because they move money.

#### Scenario: RSVP on a paid event sends the member to checkout

- **WHEN** an Active member submits the RSVP control on an event whose `member_price_cents` is greater than `0`
- **THEN** the handler SHALL return a redirect to a Stripe Checkout session rather than an "RSVP confirmed" fragment, and the member SHALL NOT yet be `Registered`

#### Scenario: RSVP to an admin-only event is refused

- **WHEN** a non-admin Active member posts `POST /portal/api/events/:id/rsvp` with the id of an `AdminOnly` event
- **THEN** the handler SHALL return the same "event not found" response an unknown id returns, no `event_attendance` row SHALL be created, and no payment or Checkout session SHALL be created

