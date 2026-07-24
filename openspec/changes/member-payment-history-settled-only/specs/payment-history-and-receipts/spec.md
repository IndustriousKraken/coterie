# payment-history-and-receipts Specification

## ADDED Requirements

### Requirement: The member payment history lists only settled payments

The member-facing payment history SHALL list only settled payments — `Completed`
and `Refunded` — and SHALL omit `Pending` and `Failed` from the member's view.
This applies to `GET /portal/payments` and its HTMX list fragment, and keeps
abandoned-checkout and transient-failure rows out of the member's history.
Admin-facing payment views SHALL be unaffected: an admin viewing a member's
payments SHALL still see every status, including `Pending` and `Failed`.
Per-payment receipts and the annual dues statement are unchanged (both already
reflect settled payments).

#### Scenario: An abandoned checkout is hidden from the member

- **WHEN** a member cancels a Stripe checkout, leaving a `Pending`-then-`Failed`
  payment row
- **THEN** that row SHALL NOT appear in the member's Payments history

#### Scenario: Completed and refunded payments are shown

- **WHEN** a member has `Completed` and `Refunded` payments
- **THEN** both SHALL appear in the member's Payments history

#### Scenario: Admins still see all statuses

- **WHEN** an admin views a member's payments
- **THEN** `Pending` and `Failed` payments SHALL still be visible in the admin view
