## ADDED Requirements

### Requirement: Two payment-recording entry points: PaymentService::record_manual and WebhookDispatcher

Payments SHALL be recorded via exactly two entry points:

- **`PaymentService::record_manual`** — for non-Stripe payments (Cash, Check, Waived, Other). The service SHALL reject `PaymentMethod::Stripe` with a `BadRequest`; Stripe payments SHALL go through the webhook flow.
- **`WebhookDispatcher::handle_*`** — for Stripe payments (PaymentIntent, CheckoutSession, Invoice). Verified-signature inbound, idempotency-claimed, dispatched per event type.

Both entry points SHALL persist via `payment_repo.create(...)`. Direct `payment_repo.create` calls from handlers or services other than these two SHALL be forbidden.

#### Scenario: record_manual rejects Stripe method

- **WHEN** a caller invokes `PaymentService::record_manual` with `PaymentMethod::Stripe`
- **THEN** the call SHALL return `BadRequest("Stripe payments are recorded via StripeClient, not record_manual")`

#### Scenario: Webhook handler is the only Stripe-payment writer

- **WHEN** a Stripe payment-succeeded event arrives
- **THEN** the webhook dispatcher's per-type handler SHALL construct the `Payment` value and call `payment_repo.create`; no other code path SHALL write Stripe payments

### Requirement: PaymentService::record_manual emits the audit-log entry

`record_manual` SHALL emit an audit-log entry via `audit_service.log` after a successful repo write, using a centralized `audit_action(method, kind)` mapping that produces the action string. The mapping SHALL be:

- `(Waived, _)` → `"waive_dues"`
- `(_, Membership)` → `"manual_payment"`
- `(_, Donation { .. })` → `"manual_donation"`
- `(_, Other)` → `"manual_other"`

Centralization SHALL prevent the four sites that previously duplicated this from drifting.

#### Scenario: Cash dues payment audits as manual_payment

- **WHEN** `record_manual` records a `(PaymentMethod::Cash, PaymentKind::Membership)` payment
- **THEN** the emitted audit row SHALL have `action = "manual_payment"`

#### Scenario: Waived dues audits as waive_dues

- **WHEN** `record_manual` records a `(PaymentMethod::Waived, PaymentKind::Membership)` payment
- **THEN** the emitted audit row SHALL have `action = "waive_dues"`

### Requirement: Membership-kind payments trigger dues-extension and reschedule (soft-fail)

When `record_manual` records a `PaymentKind::Membership` payment with a `membership_type_slug`, it SHALL:

1. Call `billing_service.extend_member_dues_by_slug` to advance `dues_paid_until`.
2. Call `billing_service.reschedule_after_payment` to update auto-renew schedule.

Both calls SHALL be soft-fail: errors are logged via `tracing` but do NOT roll back the payment row. The payment is recorded; dues extension is best-effort.

#### Scenario: Dues extension failure does not roll back payment row

- **WHEN** the dues-extension call returns an error after a successful `payment_repo.create`
- **THEN** the payment row SHALL persist; the failure SHALL be logged at error level

### Requirement: Validation at the service boundary

`record_manual` SHALL validate at entry: `amount_cents >= 0`, `amount_cents <= MAX_PAYMENT_CENTS`, member exists, and donation-campaign id exists when supplied. These checks defend against forged JSON / stale forms even though the UI normally produces valid input.

#### Scenario: Negative amount rejected

- **WHEN** `record_manual` receives `amount_cents = -100`
- **THEN** it SHALL return `BadRequest`

#### Scenario: Amount over cap rejected

- **WHEN** `record_manual` receives an amount exceeding `MAX_PAYMENT_CENTS`
- **THEN** it SHALL return `BadRequest` naming the cap

#### Scenario: Unknown member rejected

- **WHEN** `record_manual` receives a `member_id` not present in `members`
- **THEN** it SHALL return `BadRequest("member <id> not found")`

#### Scenario: Donation with stale campaign id rejected

- **WHEN** `record_manual` receives `PaymentKind::Donation { campaign_id: Some(stale_id) }` where the campaign no longer exists
- **THEN** the call SHALL return `BadRequest`; orphan donation rows SHALL NOT be created

### Requirement: Refund flow is the third write path with handler-emitted audit

`POST /portal/admin/payments/:id/refund` SHALL be a third state-changing path on payment rows. Today's implementation handles the refund (Stripe API call + ledger update) at the handler level and emits its own audit-log entry. There is no `PaymentService::refund` wrapper.

#### Scenario: Stripe-backed refund calls Stripe and updates the ledger from the handler

- **WHEN** an admin refunds a payment whose `external_id` is `Some(StripeRef::PaymentIntent(...))`
- **THEN** the handler SHALL call Stripe's refund API, update the payment row, and emit the audit-log entry directly
