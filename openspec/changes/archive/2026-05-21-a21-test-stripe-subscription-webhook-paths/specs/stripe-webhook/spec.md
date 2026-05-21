## ADDED Requirements

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
