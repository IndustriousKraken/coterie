---
changelog: skip
---

## Why

Three production Stripe webhook handlers have **zero test coverage**. They
are dispatched from `WebhookDispatcher::handle_webhook`
(`src/payments/webhook_dispatcher/mod.rs:112-170`) but, unlike the
payment-intent / charge / checkout / subscription-deleted / invoice
handlers, they have no `dispatch_*` test seam and no test exercises them:

- `handle_failed_payment` —
  `src/payments/webhook_dispatcher/payment_intent.rs:13-28`. On
  `payment_intent.payment_failed`, finds the payment by Stripe id and flips
  `status` to `PaymentStatus::Failed`. When no payment matches, it is a
  silent no-op. **Neither branch is tested.**
- `handle_expired_session` —
  `src/payments/webhook_dispatcher/checkout.rs:190-203`. On
  `checkout.session.expired`, finds the payment by session id and flips
  `status` to `Failed`. Unknown session → silent no-op. **Neither branch
  is tested.**
- `handle_subscription_updated` —
  `src/payments/webhook_dispatcher/subscription.rs:80-108`. On
  `customer.subscription.updated`, refreshes the member's stored
  `subscription_id` via `set_billing_mode`. Unknown customer → silent
  no-op. **Neither branch is tested.**

These are real billing-correctness paths: a failed payment intent or an
expired checkout that does *not* flip its row to `Failed` leaves a
phantom `Pending` payment on the ledger, and a missed
`subscription.updated` leaves a stale `subscription_id` so later invoice
events can no longer be matched to the member.

### Contract change (why spec lane, not issue lane)

Closing this gap requires a **new test-seam contract**. The existing
canon (`Requirement: Dispatcher exposes test seams …`) enumerates only
`dispatch_payment_intent_succeeded`, `dispatch_charge_refunded`,
`dispatch_subscription_deleted`, and `dispatch_checkout_session_completed`.
There is no seam for these three handlers, and they are `pub(super)` —
unreachable from an integration test without forging a signed webhook.
This change adds `dispatch_failed_payment`, `dispatch_expired_session`,
and `dispatch_subscription_updated` seams (under
`#[cfg(any(test, feature = "test-utils"))]`, mirroring the existing
convention) and adds a coverage requirement, exactly as the prior
invoice-handler coverage change did for `dispatch_invoice_*`.

## What Changes

- Add three test-seam methods on `WebhookDispatcher` (cfg-gated) that
  forward to the three handlers.
- Add a coverage requirement to the `stripe-webhook` capability naming
  the three handlers and the happy-path + unknown-target behaviors the
  tests must assert.
- Add six `#[tokio::test]` cases to `tests/stripe_webhook_test.rs`.

## Impact

- `src/payments/webhook_dispatcher/mod.rs` — three new cfg-gated
  `dispatch_*` seam methods (no production behavior change).
- `tests/stripe_webhook_test.rs` — six new test functions, reusing the
  existing harness, payload builders, and dispatcher construction.
- `openspec/specs/stripe-webhook/spec.md` (via this change's delta) — one
  new requirement.
