## Why

`src/payments/webhook_dispatcher.rs` has two handlers — `handle_invoice_paid` and `handle_invoice_payment_failed` — that are the entire billing flow for Stripe-subscription members. Every time a Stripe-managed subscription successfully bills, `invoice.paid` fires and `handle_invoice_paid` extends the member's `dues_paid_until`. Every time a charge fails, `invoice.payment_failed` fires and `handle_invoice_payment_failed` triggers `notify_subscription_payment_failed` (which emails the member and dispatches `AdminAlert`).

Neither handler has a test in `tests/stripe_webhook_test.rs`. The file covers `payment_intent.succeeded`, `charge.refunded`, `subscription.deleted`, `checkout.session.completed` — but invoice events are conspicuously absent. The architectural review missed this; an audit during deployment planning surfaced it.

This matters for the upcoming neontemple.com production migration: every imported member starts in `StripeSubscription` mode and stays there until they organically migrate. The `invoice.paid` and `invoice.payment_failed` handlers will fire for every renewal cycle of every imported member. Going to production without test coverage on the code path 100% of paying members exercise is a real risk.

## What Changes

- **Add `handle_invoice_paid` test coverage** in `tests/stripe_webhook_test.rs`:
  - `invoice_paid_extends_dues_for_stripe_subscription_member` — seed a `StripeSubscription`-mode member with a known `dues_paid_until` and a known `stripe_subscription_id`, fire a synthesized `invoice.paid` event referencing that subscription, assert the member's `dues_paid_until` advances by the membership billing period, and that a Payment row is recorded.
  - `invoice_paid_idempotency` — fire the same `invoice.paid` event twice (Stripe retry simulation), assert `dues_paid_until` only advances once.
  - `invoice_paid_for_unknown_subscription_is_noop` — fire an event for a subscription ID that doesn't match any member, assert no DB changes and a warning log.

- **Add `handle_invoice_payment_failed` test coverage**:
  - `invoice_payment_failed_dispatches_admin_alert` — seed a `StripeSubscription` member, fire `invoice.payment_failed` (non-final), assert: `notify_subscription_payment_failed` was called → AdminAlert was dispatched to the IntegrationManager (verifiable via the test IntegrationManager that records dispatched events) → no change to `dues_paid_until` → no change to `billing_mode`.
  - `invoice_payment_failed_final_attempt_softens_copy` — same but with `next_payment_attempt = None` (Stripe exhausted retries), assert `is_final = true` was passed to `notify_subscription_payment_failed`.
  - `invoice_payment_failed_for_unknown_subscription_is_noop` — symmetric noop test.

- **Out of scope**: the `subscription.updated` handler (it's just observed; the existing code logs and continues with no DB change — tests would be empty assertions). The `customer.subscription.created` handler isn't relevant for the migration since Coterie isn't creating new Stripe subscriptions.

## Capabilities

### New Capabilities

(None — adds tests to an existing capability.)

### Modified Capabilities
- `stripe-webhook`: documents the expanded test coverage requirement so future contributors don't ship handler changes for these events without tests.

## Impact

- **Code**: `tests/stripe_webhook_test.rs` grows by ~250 lines (~6 new tests, each ~40 lines with harness reuse).
- **Wire shape**: no code changes; tests only.
- **Risk**: trivial. If a test surfaces a bug in the handler, that's a discovery, not a regression introduced by this change.
- **Production timing**: Strongly recommended pre-launch. The neontemple.com members all bill through these code paths.
- **Dependency**: independent of `a20-import-billing-fields`. Both can run in either order; the autocoder will pick them up sequentially.
