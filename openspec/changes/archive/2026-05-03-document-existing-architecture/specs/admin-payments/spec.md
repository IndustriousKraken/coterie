## ADDED Requirements

### Requirement: Admin records manual payments via the payment service

Manual payment recording SHALL go through the payment service so that audit-log entries and integration events match the Stripe-webhook path. Admins SHALL access this via `GET/POST /portal/admin/members/:id/record-payment`.

#### Scenario: Manual recording emits the same side effects as a webhook payment

- **WHEN** an admin records a manual cash payment
- **THEN** the resulting audit-log row, integration events, and dues-paid-until update SHALL match those that the corresponding Stripe-webhook path would emit

### Requirement: Refunds are recorded against the original payment

`POST /portal/admin/payments/:id/refund` SHALL record a refund against the original payment row and trigger the corresponding Stripe API call (or skip it for manual-payment refunds, recording only). The service SHALL emit an audit-log row.

#### Scenario: Refund without Stripe id records ledger entry only

- **WHEN** an admin refunds a manual (non-Stripe) payment
- **THEN** the system SHALL record the refund in the ledger and emit an audit-log entry without calling Stripe

#### Scenario: Refund with Stripe id calls Stripe and records the response

- **WHEN** an admin refunds a Stripe-backed payment
- **THEN** the system SHALL call Stripe to issue the refund, record the resulting refund id, and emit an audit-log entry
