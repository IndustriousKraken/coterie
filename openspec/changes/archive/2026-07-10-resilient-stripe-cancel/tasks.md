# Tasks

## 1. Preserve the Stripe error category

- [x] 1.1 Replace the opaque flatten in the cancel path: instead of routing
  `Subscription::delete` through `timed` (which maps every
  `stripe::StripeError` to one `AppError::External` string), match the
  `StripeError` variant and classify it as one of: response-parse error
  (serde), Stripe API error (a returned error status), or transport error
  (network / timeout).

## 2. Tolerate an unparseable success response on cancel

- [x] 2.1 In `delete_subscription`, treat a response-parse error as success
  (the DELETE carries no request body, so the parse failure is on the
  response — Stripe already cancelled the sub). Log a warning: cancel
  processed, response unparseable, `customer.subscription.deleted` webhook
  will reconcile.
- [x] 2.2 Return `Err` only for a genuine Stripe API error or a transport
  error, so a real failure is still surfaced.

## 3. Stop the false rollback / webhook race

- [x] 3.1 In `disable_auto_renew` and `migrate_to_coterie_managed`
  (`src/service/billing_service/auto_renew.rs`), rely on the resilient
  `delete_subscription`: a successful (or parse-tolerated) cancel no longer
  rolls back local billing mode, so it stops racing the
  `customer.subscription.deleted` webhook.

## 4. Investigate the root deserialization mismatch

- [x] 4.1 Determine whether the failure is a Stripe API-version mismatch
  (account default version vs what async-stripe 0.39 expects). If so, pin the
  client's API version in `RealStripeGateway::new` so the response parses at
  the source. Verify this does not break other endpoints.

## 5. Tests (offline, via FakeStripeGateway — no real keys)

- [x] 5.1 Add a seam to `FakeStripeGateway` so `delete_subscription` can be
  made to return a response-parse error.
- [x] 5.2 Assert that with that seam, `disable_auto_renew` returns success,
  does NOT roll back billing mode, and surfaces no error to the caller.
- [x] 5.3 Assert that a genuine API error from the fake DOES surface as a
  failure (so the tolerance is scoped to parse errors only).
