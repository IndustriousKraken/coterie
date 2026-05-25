# payment-recording Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: PaymentService::record_manual emits the audit-log entry

`record_manual` SHALL emit an audit-log entry via `audit_service.log` after a
successful repo write, using a centralized `audit_action(method, kind)`
mapping that produces the action string. The mapping SHALL be:

- `(Waived, _)` → `"waive_dues"`
- `(_, Membership)` → `"manual_payment"`
- `(_, Donation { .. })` → `"manual_donation"`
- `(_, Other)` → `"manual_other"`

Centralization SHALL prevent the four sites that previously duplicated this
from drifting.

#### Scenario: Cash dues payment audits as manual_payment

- **WHEN** `record_manual` records a `(PaymentMethod::Cash,
  PaymentKind::Membership)` payment
- **THEN** the emitted audit row SHALL have `action = "manual_payment"`

#### Scenario: Waived dues audits as waive_dues

- **WHEN** `record_manual` records a `(PaymentMethod::Waived,
  PaymentKind::Membership)` payment
- **THEN** the emitted audit row SHALL have `action = "waive_dues"`

#### Scenario: Cash donation audits as manual_donation

- **WHEN** `record_manual` records a `(PaymentMethod::Cash,
  PaymentKind::Donation { .. })` payment
- **THEN** the emitted audit row SHALL have `action = "manual_donation"`

### Requirement: Membership-kind payments trigger dues-extension and reschedule (soft-fail)

When `record_manual` records a `PaymentKind::Membership` payment with a `membership_type_slug`, it SHALL:

1. Call `billing_service.extend_member_dues_by_slug` to advance `dues_paid_until`.
2. Call `billing_service.reschedule_after_payment` to update auto-renew schedule.

Both calls SHALL be soft-fail: errors are logged via `tracing` but do NOT roll back the payment row. The payment is recorded; dues extension is best-effort.

#### Scenario: Dues extension failure does not roll back payment row

- **WHEN** the dues-extension call returns an error after a successful `payment_repo.create`
- **THEN** the payment row SHALL persist; the failure SHALL be logged at error level

### Requirement: Validation at the service boundary

`record_manual` SHALL validate at entry: `amount_cents >= 0`,
`amount_cents <= MAX_PAYMENT_CENTS`, member exists, and donation-campaign id
exists when supplied. These checks defend against forged JSON / stale forms
even though the UI normally produces valid input.

#### Scenario: Negative amount rejected

- **WHEN** `record_manual` receives `amount_cents = -100`
- **THEN** it SHALL return `BadRequest` AND the `payments` table SHALL
  remain empty (no row was persisted before the guard fired)

#### Scenario: Amount over cap rejected

- **WHEN** `record_manual` receives an amount exceeding `MAX_PAYMENT_CENTS`
- **THEN** it SHALL return `BadRequest` whose message names the cap
  in whole dollars

#### Scenario: Unknown member rejected

- **WHEN** `record_manual` receives a `member_id` not present in `members`
- **THEN** it SHALL return `BadRequest` whose message includes the
  unknown id

#### Scenario: Donation with stale campaign id rejected

- **WHEN** `record_manual` receives `PaymentKind::Donation { campaign_id:
  Some(stale_id) }` where the campaign no longer exists
- **THEN** the call SHALL return `BadRequest` AND the `payments` table
  SHALL contain no row for this attempt (no orphan donation row is created)

### Requirement: Refund flow is the third write path with handler-emitted audit

`POST /portal/admin/payments/:id/refund` SHALL be a third state-changing path on payment rows. Today's implementation handles the refund (Stripe API call + ledger update) at the handler level and emits its own audit-log entry. There is no `PaymentService::refund` wrapper.

#### Scenario: Stripe-backed refund calls Stripe and updates the ledger from the handler

- **WHEN** an admin refunds a payment whose `external_id` is `Some(StripeRef::PaymentIntent(...))`
- **THEN** the handler SHALL call Stripe's refund API, update the payment row, and emit the audit-log entry directly

### Requirement: Payment-recording entry points are explicitly enumerated

Payments SHALL be recorded via exactly three entry points:

- **`PaymentService::record_manual`** — for non-Stripe payments (Cash, Check, Waived, Other). Operator-initiated via the admin UI. The service SHALL reject `PaymentMethod::Stripe` with a `BadRequest`; Stripe payments SHALL go through one of the other two entry points.
- **`WebhookDispatcher::handle_*`** — for Stripe-initiated events: customer paid an invoice, customer completed a checkout session, payment-intent succeeded. Inbound to Coterie, verified-signature, idempotency-claimed, dispatched per event type.
- **`BillingService::process_scheduled_payment`** — for Coterie-initiated auto-renew charges against a saved card. The scheduled payment row is the Coterie-side trigger; the Stripe charge is a direct API call (not a webhook); on charge success, the `Payment` row is created from the charge result.

All three entry points SHALL persist via `payment_repo.create(...)`. Direct `payment_repo.create` calls from handlers or services OTHER than these three SHALL be forbidden. Adding a fourth entry point requires updating this spec.

Why three, not two: `BillingService::process_scheduled_payment` doesn't fit either of the other two — it's not operator-initiated (so not `record_manual`) and there's no inbound webhook (the charge is initiated by Coterie's scheduler, not Stripe). The third entry point reflects this legitimately distinct shape.

#### Scenario: record_manual rejects Stripe method

- **WHEN** a caller invokes `PaymentService::record_manual` with `PaymentMethod::Stripe`
- **THEN** the call SHALL return `BadRequest("Stripe payments are recorded via StripeClient, not record_manual")`

#### Scenario: Webhook handler is the only writer for Stripe-inbound events

- **WHEN** a Stripe payment-succeeded webhook event arrives
- **THEN** the webhook dispatcher's per-type handler SHALL construct the `Payment` value and call `payment_repo.create`; no other code path SHALL write payments from Stripe-inbound events

#### Scenario: Auto-renew charges write payments via BillingService

- **WHEN** a scheduled payment is processed and the saved-card charge succeeds
- **THEN** `BillingService::process_scheduled_payment` SHALL construct the `Payment` value and call `payment_repo.create`; the resulting payment row SHALL be linked to the scheduled-payment row and audited

#### Scenario: A fourth entry point requires a spec amendment

- **WHEN** a contributor adds a new code path that records a payment outside the three listed entry points
- **THEN** the PR SHALL be rejected pending an amendment to this requirement listing the new entry point; the rule exists to prevent accidental audit/event-skipping by ad-hoc payment-row writers

