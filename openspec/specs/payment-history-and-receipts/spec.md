# payment-history-and-receipts Specification

## Purpose
TBD - created by archiving change import-stripe-payment-history. Update Purpose after archive.
## Requirements
### Requirement: Payment history is backfilled from Stripe

The system SHALL backfill historical payments from Stripe: for each member
with a Stripe customer, it SHALL create a local payment record per historical
charge or invoice. The backfill SHALL be idempotent, keyed on the Stripe id
and relying on the existing per-Stripe-id uniqueness, so that a re-run imports
nothing new and never double-counts. The backfill is the fourth
payment-recording entry point (see the MODIFIED `payment-recording`
requirement): it persists via `payment_repo.create` and emits its own
`import_payment` and `import_payments_batch` audit rows, so it does not skip
the audit trail; it does NOT extend dues or dispatch integration events.

#### Scenario: Historical charges become local payment rows

- **WHEN** the backfill runs for a member whose Stripe customer has past
  paid invoices
- **THEN** a local payment record SHALL exist for each, with amount, currency,
  date, and the Stripe id

#### Scenario: Re-running the backfill imports nothing new

- **WHEN** the backfill runs a second time
- **THEN** no duplicate payment rows SHALL be created

### Requirement: Saved cards are backfilled from Stripe

The system SHALL backfill saved cards from Stripe: for each member's Stripe
customer, it SHALL insert a local card record per attached card, skipping any
whose card fingerprint already exists for that member so a card is not stored
twice. The member's default card SHALL be set from the Stripe customer's
default payment method when one is known. This backfill imports references to
cards ALREADY attached to the Stripe customer and therefore does not use the
SetupIntent flow (see the MODIFIED `saved-card-management` requirement); it
receives no raw card numbers, only `pm_*` ids and display metadata from
Stripe.

#### Scenario: A member's Stripe card becomes visible in Coterie

- **WHEN** the backfill runs for a member with a card attached to their Stripe
  customer
- **THEN** that card SHALL appear in the member's Coterie saved-cards list

#### Scenario: A duplicate card is not re-inserted

- **WHEN** the backfill encounters a card whose fingerprint already exists for
  the member
- **THEN** no second local card record SHALL be created for it

#### Scenario: Coterie's default matches Stripe's default, not the first card imported

- **WHEN** the backfill imports cards for a member whose Stripe customer has a
  default payment method that is not the first card imported
- **THEN** the member's Coterie default SHALL be the card Stripe marks as
  default, so a member who converts to Coterie-managed keeps being charged on
  the same card Stripe was already using, not a different one

### Requirement: Members can view and download a per-payment receipt

The system SHALL let a member view and download a receipt for any of their
recorded payments, rendered from the organization receipt settings. An admin
SHALL be able to view the same receipts for any member.

#### Scenario: A receipt is available for a recorded payment

- **WHEN** a member opens one of their payments
- **THEN** a receipt SHALL be viewable and downloadable, showing the org
  details, amount, date, and what the payment was for

### Requirement: An annual dues statement is available per member per year

The system SHALL provide a per-member, per-calendar-year statement that sums
the dues that member paid in that year, in a printable and downloadable form
suitable for a tax write-off. A member SHALL be able to pull their own
statement and an admin SHALL be able to pull it for any member.

#### Scenario: The annual total matches the recorded payments

- **WHEN** a member requests their statement for a given year
- **THEN** the total SHALL equal the sum of that member's dues payments dated
  within that calendar year

### Requirement: A receipt is emailed on each new charge when email is configured

The system SHALL email the member a receipt when a live (non-backfill) payment is recorded — a
subscription invoice arriving by webhook, or a Coterie-initiated charge —
provided an email provider is configured. When no email provider is
configured the send SHALL be skipped silently, the receipt SHALL remain
viewable in the portal, and the payment SHALL NOT fail on account of email.
The one-time Stripe payment-history backfill SHALL NOT trigger receipt emails; it imports already-settled
historical charges and only persists the payment row and its
`import_payment`/`import_payments_batch` audit rows.

#### Scenario: Receipt email is sent when email is configured

- **WHEN** a live (non-backfill) payment is recorded and an email provider is configured
- **THEN** a receipt email SHALL be sent to the member

#### Scenario: The payment-history backfill does not send receipts

- **WHEN** the one-time Stripe payment-history backfill imports a settled historical charge and an email provider is configured
- **THEN** no receipt email SHALL be sent; the payment row and its `import_payment`/`import_payments_batch` audit rows SHALL be persisted without dispatching notifications

#### Scenario: Payment still succeeds when email is unconfigured

- **WHEN** a payment is recorded and no email provider is configured
- **THEN** no email SHALL be attempted, the payment SHALL still be recorded,
  and the receipt SHALL still be viewable in the portal

### Requirement: The member payment history lists only settled payments

The member-facing payment history SHALL list only settled payments — `Completed`
and `Refunded` — and SHALL omit `Pending` and `Failed` from the member's view.
This applies to **every member-facing surface that lists payments**, including
`GET /portal/payments` and its HTMX list fragment and the dashboard's recent-payments
fragment, and keeps abandoned-checkout and transient-failure rows out of the
member's history.
Admin-facing payment views SHALL be unaffected: an admin viewing a member's
payments SHALL still see every status, including `Pending` and `Failed`.
Per-payment receipts and the annual dues statement are unchanged (both already
reflect settled payments).

The rule is a property of the member's view, not of a route. Naming one page left
the dashboard fragment outside it, and an implementation that filtered exactly
where the requirement pointed produced a portal whose two payment lists disagreed:
an abandoned checkout appeared on the dashboard and vanished on the page it linked
to. A member reading their own history is checking whether they were charged, and
two answers to that question is worse than either.

Where a surface limits how many payments it shows, the filter SHALL be applied
**before** the limit. Truncating first and filtering after would show a member
with several unsettled rows an empty list while settled payments sat just outside
the window — a different defect wearing this one's fix.

The decision SHALL have one implementation shared by every surface. The defect
this requirement corrects is a surface that did not use the existing shared
predicate, so a second copy of that predicate SHALL NOT be introduced.

#### Scenario: An abandoned checkout is hidden from the member

- **WHEN** a member cancels a Stripe checkout, leaving a `Pending`-then-`Failed`
  payment row
- **THEN** that row SHALL NOT appear in the member's Payments history

#### Scenario: An abandoned checkout is hidden on the dashboard too

- **WHEN** that same member views the dashboard's recent-payments fragment
- **THEN** the row SHALL NOT appear there either, and the two surfaces SHALL agree

#### Scenario: A limited list filters before it truncates

- **WHEN** a member's most recent payments are unsettled and older settled
  payments exist, on a surface that shows a fixed number
- **THEN** the settled payments SHALL be shown, rather than an empty list

#### Scenario: Completed and refunded payments are shown

- **WHEN** a member has `Completed` and `Refunded` payments
- **THEN** both SHALL appear in the member's Payments history

#### Scenario: Admins still see all statuses

- **WHEN** an admin views a member's payments
- **THEN** `Pending` and `Failed` payments SHALL still be visible in the admin view

