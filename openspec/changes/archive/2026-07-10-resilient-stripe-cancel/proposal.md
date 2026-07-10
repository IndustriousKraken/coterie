# resilient-stripe-cancel

## Why

Cancelling a Stripe subscription reports a failure even when it succeeds.
Clicking "Cancel subscription" (or "Switch to Coterie auto-renew", which
cancels the legacy sub) shows a red **"upstream error"**, yet the
subscription is actually cancelled.

Root cause, confirmed from production logs:

```
ERROR External service error: Stripe cancel failed:
      Stripe error: error serializing or deserializing a request
...next line: customer.subscription.deleted webhook processed, member -> manual
```

`gateway.delete_subscription` (`src/payments/gateway.rs:545`) calls
async-stripe 0.39 `Subscription::delete`. Stripe **accepts the DELETE and
cancels the subscription**, then returns the cancelled Subscription object —
which async-stripe 0.39 **fails to deserialize**. The `timed` wrapper
(`gateway.rs:52`) flattens every `stripe::StripeError` variant into one
opaque `AppError::External` string, so the caller cannot tell a
response-parse failure from a real API failure. `disable_auto_renew` /
`migrate_to_coterie_managed` then treat the parse error as a failure, roll
back the local billing mode, and surface "upstream error" — while the
`customer.subscription.deleted` webhook simultaneously reconciles the member
to `manual`, leaving the two racing and the member in an inconsistent state.

Key insight that makes a safe fix possible: **`Subscription::delete` sends no
request body**, so a serialize/deserialize error on that specific call is
*always* the response — i.e. Stripe already processed the cancel. Combined
with the fact that the `customer.subscription.deleted` webhook is the
authoritative reconciler of member state, a response-parse error on cancel
is cosmetic, not a real failure.

## What Changes

- The gateway SHALL stop flattening Stripe errors into one opaque string for
  the cancel path: it SHALL distinguish a **response (de)serialize error**
  from a **Stripe API error** (a returned error status) and from a
  **transport error** (network / timeout).
- `delete_subscription` SHALL treat a response-parse error as **success**
  (logging a warning that the cancel was processed but the response was
  unparseable, and the webhook will reconcile), and SHALL return `Err` only
  for a genuine API error or transport error.
- The cancel callers (`disable_auto_renew`, `migrate_to_coterie_managed`)
  therefore no longer roll back or show "upstream error" on a successful
  cancel, and no longer race the `customer.subscription.deleted` webhook.
- Investigate and, if confirmed, **pin the Stripe API version** the client
  sends to the version async-stripe 0.39 expects, to fix the deserialization
  at its source and prevent the same class of failure on other endpoints.

## Impact

- **Spec:** `recurring-billing` — ADDED requirement that subscription cancel
  tolerates an unparseable success response and does not false-fail or
  roll back.
- **Code:** `src/payments/gateway.rs` (a cancel-specific error match instead
  of the opaque `timed` flatten; or a `timed` variant that preserves the
  `StripeError` category); `src/service/billing_service/auto_renew.rs`
  (`disable_auto_renew` / `migrate_to_coterie_managed` no longer roll back on
  a parse-only error); optional client API-version pin in `RealStripeGateway`.
- **Testing:** uses the existing `FakeStripeGateway` seam (feature
  `test-utils`) — NO real Stripe keys required. Add a seam so the fake can
  return a response-parse error for `delete_subscription`, and assert the
  caller treats it as success with no rollback. The real async-stripe
  deserialization can be spot-checked once against Stripe test mode by a
  human, but the fix's behavior is fully unit-testable offline.
- **No change** to the webhook path (already correct) or to any wire format.
