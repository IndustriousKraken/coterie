# stripe-webhook Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Webhook is HMAC-authed via Stripe-Signature

`POST /api/payments/webhook/stripe` SHALL be CSRF-exempt and SHALL authenticate each delivery by verifying the `Stripe-Signature` header against the configured webhook secret. Requests with an invalid signature SHALL be rejected.

#### Scenario: Valid signature is accepted

- **WHEN** a Stripe POST arrives with a valid `Stripe-Signature` header for the configured secret
- **THEN** the handler SHALL parse the event and dispatch it

#### Scenario: Invalid signature is rejected

- **WHEN** the signature verification fails
- **THEN** the handler SHALL return 400 (or 401) without processing the event payload

#### Scenario: Endpoint is in CSRF_EXEMPT_PATHS

- **WHEN** the application boots
- **THEN** `POST /api/payments/webhook/stripe` SHALL be in the static `CSRF_EXEMPT_PATHS` list

### Requirement: Event processing is idempotent via atomic claim

The webhook dispatcher SHALL claim each `event_id` in the `processed_stripe_events` table BEFORE processing using `INSERT OR IGNORE`. The repository's `claim(event_id, event_type)` SHALL return `true` if the row was inserted (first time) and `false` if a duplicate. The dispatcher SHALL bail early when claim returns `false`.

#### Scenario: Duplicate delivery is a no-op

- **WHEN** Stripe retries an event with the same `event_id`
- **THEN** `claim` SHALL return `false`; the dispatcher SHALL log "Skipping already-processed Stripe event" and return `Ok(())` without invoking handlers

#### Scenario: Concurrent claim is single-flight

- **WHEN** two workers simultaneously receive the same event id
- **THEN** SQLite's per-row write conflict handling on `INSERT OR IGNORE` SHALL ensure exactly one claim succeeds; the other returns `false`

### Requirement: Failed processing releases the claim for retry

If event handling fails after the claim has been made, the dispatcher SHALL release the claim by deleting the `processed_stripe_events` row so Stripe's next retry has a chance to succeed.

#### Scenario: Transient handler failure is retried on next delivery

- **WHEN** event handling errors (DB transient, downstream service unavailable)
- **THEN** the dispatcher SHALL call `processed_events_repo.release(event_id)` to remove the claim, log the rollback, and return the error so Stripe retries

### Requirement: Dispatcher exposes test seams under cfg(test, feature = "test-utils")

`WebhookDispatcher` SHALL expose the following `dispatch_*` methods under `#[cfg(any(test, feature = "test-utils"))]` so integration tests can call into the dispatcher directly without forging signatures:

- `dispatch_payment_intent_succeeded`
- `dispatch_charge_refunded`
- `dispatch_subscription_deleted`
- `dispatch_checkout_session_completed`

#### Scenario: Tests dispatch events directly

- **WHEN** a test in `tests/stripe_webhook_test.rs` simulates a payment-succeeded event
- **THEN** it SHALL call `dispatch_payment_intent_succeeded` with a constructed payload, bypassing signature verification

#### Scenario: Test seams are not compiled in release builds without the feature

- **WHEN** the application is built without `--features test-utils`
- **THEN** the `dispatch_*` methods SHALL NOT be present in the binary; production callers SHALL be forced through `handle_webhook` (which verifies signatures)

### Requirement: Side effects for webhook events match admin-recorded payments

Side-effects (audit-log entries, integration events, dues-paid-until updates) for webhook-recorded payments SHALL match those for admin-recorded payments when describing the same business action. Both paths SHALL go through the payment service.

#### Scenario: Webhook payment and manual payment write equivalent audit rows

- **WHEN** a payment is recorded via the webhook AND an equivalent payment is recorded by an admin
- **THEN** the audit-log rows and integration events SHALL describe the same business action with the same target/payload shape (differing only in actor and source fields)

### Requirement: Invoice-event webhook handlers have integration test coverage

`tests/stripe_webhook_test.rs` SHALL include test coverage for both `handle_invoice_paid` and `handle_invoice_payment_failed`. The tests SHALL exercise:

- The happy path (event matches a known member, expected DB state change occurs).
- Idempotency (same event fired twice produces single DB state change).
- Graceful handling of events for subscriptions Coterie doesn't know about (no panic, no spurious DB writes).
- For `handle_invoice_payment_failed`: the `AdminAlert` dispatch is verified — this is the production lifeline for operators to learn about billing failures without polling logs.

Adding handler changes for these event types in the future SHALL include updates to the relevant test, not just a "trust me" commit.

#### Scenario: invoice.paid extends dues for known subscription

- **WHEN** an `invoice.paid` event is dispatched for a subscription ID that matches an active `StripeSubscription`-mode member
- **THEN** the test SHALL assert that the member's `dues_paid_until` advances by the membership billing period AND a Payment row is recorded with the event's payment_intent ID

#### Scenario: invoice.paid is idempotent under Stripe retry

- **WHEN** the same `invoice.paid` event is dispatched twice (simulating Stripe's at-least-once delivery semantics)
- **THEN** the test SHALL assert that `dues_paid_until` advances exactly once, not twice

#### Scenario: invoice.payment_failed dispatches AdminAlert

- **WHEN** an `invoice.payment_failed` event is dispatched for a known `StripeSubscription`-mode member
- **THEN** the test SHALL assert that `IntegrationEvent::AdminAlert` was dispatched to the IntegrationManager (verifiable via test harness recording the dispatched events)

#### Scenario: invoice.payment_failed final-attempt passes is_final=true

- **WHEN** an `invoice.payment_failed` event has `next_payment_attempt = None` (Stripe exhausted retries)
- **THEN** the test SHALL assert that `Notifications::notify_subscription_payment_failed` was called with `is_final = true`

#### Scenario: Invoice events for unknown subscription_id are noops

- **WHEN** an `invoice.paid` or `invoice.payment_failed` event references a subscription ID that doesn't map to any known member
- **THEN** the test SHALL assert no DB state change AND no panic AND no AdminAlert dispatch

