## RENAMED Requirements

- FROM: `### Requirement: Two payment-recording entry points: PaymentService::record_manual and WebhookDispatcher`
- TO: `### Requirement: Payment-recording entry points are explicitly enumerated`

## MODIFIED Requirements

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
