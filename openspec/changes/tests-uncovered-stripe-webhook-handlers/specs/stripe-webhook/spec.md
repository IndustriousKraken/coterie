## ADDED Requirements

### Requirement: Lifecycle webhook handlers have integration test coverage

`tests/stripe_webhook_test.rs` SHALL include coverage for the three
lifecycle webhook handlers that flip terminal payment state or refresh
cached subscription data: `handle_failed_payment` (dispatched for
`payment_intent.payment_failed`), `handle_expired_session` (dispatched for
`checkout.session.expired`), and `handle_subscription_updated` (dispatched
for `customer.subscription.updated`).

To make these handlers reachable from integration tests without forging a
signed webhook, `WebhookDispatcher` SHALL expose `dispatch_failed_payment`,
`dispatch_expired_session`, and `dispatch_subscription_updated` methods
under `#[cfg(any(test, feature = "test-utils"))]`, following the same
convention as the existing `dispatch_*` seams. The tests SHALL exercise,
for each handler, both the matching-target path (the expected DB state
change occurs) and the unknown-target path (no panic, no spurious DB
write).

Adding handler changes for these event types in the future SHALL include
updates to the relevant test, not a "trust me" commit.

#### Scenario: payment_intent.payment_failed flips the matching payment to Failed

- **WHEN** `dispatch_failed_payment` is called with the Stripe payment id of an existing `Pending` payment whose `external_id` is `StripeRef::PaymentIntent`
- **THEN** the test SHALL assert the payment row's `status` becomes `PaymentStatus::Failed`

#### Scenario: payment_intent.payment_failed for an unknown payment id is a no-op

- **WHEN** `dispatch_failed_payment` is called with a Stripe payment id that matches no payment row
- **THEN** the test SHALL assert the call returns `Ok(())` with no panic and no payment row status change

#### Scenario: checkout.session.expired flips the matching payment to Failed

- **WHEN** `dispatch_expired_session` is called with a `CheckoutSession` whose id matches an existing `Pending` payment whose `external_id` is `StripeRef::CheckoutSession`
- **THEN** the test SHALL assert the payment row's `status` becomes `PaymentStatus::Failed`

#### Scenario: checkout.session.expired for an unknown session id is a no-op

- **WHEN** `dispatch_expired_session` is called with a `CheckoutSession` whose id matches no payment row
- **THEN** the test SHALL assert the call returns `Ok(())` with no panic and no payment row status change

#### Scenario: customer.subscription.updated refreshes the stored subscription id

- **WHEN** `dispatch_subscription_updated` is called with a `Subscription` whose customer id matches a known member and whose subscription id differs from the member's stored value
- **THEN** the test SHALL assert the member row's stored `subscription_id` is updated to the new value

#### Scenario: customer.subscription.updated for an unknown customer is a no-op

- **WHEN** `dispatch_subscription_updated` is called with a `Subscription` whose customer id maps to no member
- **THEN** the test SHALL assert the call returns `Ok(())` with no panic and no member row mutation
