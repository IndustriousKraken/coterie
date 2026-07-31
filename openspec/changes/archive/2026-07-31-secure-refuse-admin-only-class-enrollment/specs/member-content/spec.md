# member-content Specification

## MODIFIED Requirements

### Requirement: Members can RSVP to events

`POST /portal/api/events/:id/rsvp` and `POST /portal/api/events/:id/cancel` SHALL allow Active/Honorary members to manage their RSVP. For a **free** event (`member_price_cents = 0`) the handlers SHALL call `event_repo.register_attendance` / `cancel_attendance` and return an updated HTMX button fragment, exactly as before.

For a **paid** event (`member_price_cents > 0`) the RSVP handler SHALL NOT register the member directly. It SHALL instead route through the paid-registration path — claiming the seat as `PendingPayment` and returning a redirect to Stripe Checkout — per the `paid-events` capability. The member becomes `Registered` only when the checkout-completion webhook confirms payment.

The button label SHALL reflect which path applies, so a member can tell before clicking whether the control registers them or sends them to pay.

Both handlers SHALL refuse an event the requesting member may not see — an `AdminOnly` event requested by a non-admin — and SHALL answer with the same "event not found" response an unknown id produces. A distinguishable rejection would let a member confirm which admin-only event ids exist, which is the disclosure the level exists to prevent.

The member class-enrollment endpoint `POST /portal/api/series/:id/enroll` SHALL apply the same refusal at series scope. Because a series row carries no visibility of its own, the endpoint SHALL resolve the visibility decision against one of the series' occurrences using the same domain rule the event handlers use, and SHALL treat a series with no occurrences as not visible. A refused enrollment SHALL answer with the same "class not found" response an unknown series id produces, and SHALL create no `series_enrollment` row, no `event_attendance` row, no payment, and no Checkout session — in particular the class's title SHALL NOT be disclosed, whether in the response or as a Stripe Checkout line item.

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

#### Scenario: Enrolling in an admin-only class is refused

- **WHEN** a non-admin Active member posts `POST /portal/api/series/:id/enroll` with the id of a series whose occurrences are `AdminOnly`
- **THEN** the handler SHALL return the same "class not found" response an unknown series id returns, the class title SHALL NOT appear in the response, and no `series_enrollment` row, no `event_attendance` row, no payment, and no Checkout session SHALL be created

#### Scenario: An admin can still enroll in an admin-only class

- **WHEN** a member with `is_admin` posts `POST /portal/api/series/:id/enroll` for the same `AdminOnly` series
- **THEN** the enrollment SHALL proceed exactly as it does for a members-only class, per the `paid-events` capability
