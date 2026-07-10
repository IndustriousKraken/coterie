# payment-history-and-receipts Specification (delta)

## ADDED Requirements

### Requirement: Payment history is backfilled from Stripe

The system SHALL backfill historical payments from Stripe: for each member
with a Stripe customer, it SHALL create a local payment record per historical
charge or invoice. The backfill SHALL be idempotent, keyed on the Stripe id
and relying on the existing per-Stripe-id uniqueness, so that a re-run imports
nothing new and never double-counts.

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
default payment method when one is known.

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

The system SHALL email the member a receipt when a payment is recorded — a
subscription invoice arriving by webhook, or a Coterie-initiated charge —
provided an email provider is configured. When no email provider is
configured the send SHALL be skipped silently, the receipt SHALL remain
viewable in the portal, and the payment SHALL NOT fail on account of email.

#### Scenario: Receipt email is sent when email is configured

- **WHEN** a payment is recorded and an email provider is configured
- **THEN** a receipt email SHALL be sent to the member

#### Scenario: Payment still succeeds when email is unconfigured

- **WHEN** a payment is recorded and no email provider is configured
- **THEN** no email SHALL be attempted, the payment SHALL still be recorded,
  and the receipt SHALL still be viewable in the portal
