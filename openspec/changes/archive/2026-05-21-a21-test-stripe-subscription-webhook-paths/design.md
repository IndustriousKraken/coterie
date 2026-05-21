## Context

`tests/stripe_webhook_test.rs` was written alongside the BillingService split work (Tier 3.5 in ROADMAP.md). It deliberately covered the high-risk paths the BillingService refactor was about to touch — payment-intent retries, refund echoes, subscription deletion. The `invoice.*` family of events wasn't on that scope-card because the BillingService refactor didn't change those paths.

That left a gap: the `invoice.paid` / `invoice.payment_failed` handlers have shipped without dedicated tests since they were originally written. They're working in staging (we know this empirically from the autocoder's many test runs against real-Stripe-style fixtures), but "working empirically" isn't the same as "regression-protected."

Going to production, the Coterie-managed billing path has tests; the Stripe-managed billing path (which is what 100% of imported members will use initially) doesn't. Bringing the test coverage to parity before launch is cheap insurance.

## Goals / Non-Goals

**Goals:**
- Both invoice handlers (`handle_invoice_paid` and `handle_invoice_payment_failed`) have dedicated tests covering the happy path, idempotency, and the unknown-subscription noop case.
- `handle_invoice_payment_failed` tests verify that the AdminAlert IS dispatched — this is the user's "I don't want to SSH to know billing's broken" lifeline.
- New tests use the existing `build_harness` + `FakeStripeGateway` infrastructure so they fit naturally next to the existing tests.

**Non-Goals:**
- Testing `subscription.updated`. The handler is a no-op log; testing it would assert nothing meaningful.
- Testing `customer.subscription.created`. Coterie doesn't create new Stripe subs, so this event shouldn't arrive in production.
- Testing the email-template rendering itself. That's covered (or should be) in the email templates' own tests.
- Refactoring `handle_invoice_paid` or `handle_invoice_payment_failed`. This change is tests only.

## Decisions

### D1. Use the existing `build_harness` and `Harness` shape

The existing tests in `tests/stripe_webhook_test.rs` build a harness via `build_harness()` that wires up an in-memory SQLite pool + `FakeStripeGateway` + a `WebhookDispatcher`. New tests follow the same shape — no new infrastructure needed.

### D2. Synthesize stripe::Invoice values for the test events

The existing tests synthesize `stripe::Subscription` and `stripe::Charge` values directly (using the stripe-rs crate's structs with manually populated fields). The new tests do the same for `stripe::Invoice` — set the subscription_id, period_end (for the dues-extension calculation), status, etc.

If `stripe::Invoice` has so many fields that this becomes painful, fall back to a JSON-string round-trip: build a JSON blob matching Stripe's payload, parse via `serde_json::from_str::<stripe::Invoice>(...)`. The existing tests use this trick where convenient.

### D3. Verify the AdminAlert dispatch via the test IntegrationManager

The harness wires a test `IntegrationManager`. To verify `AdminAlert` is dispatched on `invoice.payment_failed`, the test inspects the IntegrationManager's dispatched-events log after the handler runs.

If the existing `IntegrationManager` doesn't currently record dispatched events for test inspection, this change extends it with a small recording shim — `#[cfg(any(test, feature = "test-utils"))]`-gated. The shim records every dispatched event in a `Vec<IntegrationEvent>` that tests can iterate.

Alternatively, the harness wires a test `Integration` impl that just records events. Either works; pick whichever fits the existing harness shape with less ceremony.

### D4. Idempotency test uses the same event twice

Stripe retries deliver an identical event payload. The idempotency test fires the same `invoice.paid` event twice. The handler should write the same DB state in both cases (no double-extension of `dues_paid_until`).

The handler's idempotency mechanism: looking at the existing code, `handle_invoice_paid` creates a Payment row keyed off the Stripe invoice ID, and the `payment_repo.create` should fail on duplicate (or the handler should detect the existing row and skip). Verify the actual mechanism during implementation and adjust the test to assert on whichever shape applies.

### D5. Unknown-subscription noop tests use a sub_id that's not in the DB

Synthesize the event with `subscription = "sub_DOES_NOT_EXIST"`. The handler's lookup returns no member; expected behavior is to log and return Ok. The test asserts no panic, no DB writes, and (ideally) a `tracing::warn!` message was emitted — but capturing tracing output in tests requires a test subscriber, which is over-engineering for v1. Settle for asserting on the DB state and the result type.

### D6. New tests live alongside existing ones in `tests/stripe_webhook_test.rs`

Don't split into a new file. The existing webhook tests already share fixtures, harness types, and helper functions. Co-location keeps the harness consolidated.

## Risks / Trade-offs

- **Risk**: writing the tests surfaces an actual bug in the handler. → That's a good outcome — better to find it pre-launch via a test than post-launch via a customer report. Fix in the same PR if minor; spec a separate change if larger.
- **Risk**: `stripe::Invoice` is awkward to synthesize. → Mitigation: fall back to JSON round-trip if direct struct construction is painful.
- **Trade-off**: ~250 lines of new test code for handlers that have been working in staging. Worth it given the production risk profile.

## Migration Plan

Single PR.

1. Skim the existing `tests/stripe_webhook_test.rs` to understand the harness shape and where to drop new tests.
2. Write `invoice_paid_extends_dues_for_stripe_subscription_member` first — establishes the synthesized-Invoice pattern.
3. Write `invoice_paid_idempotency` — confirms the handler's dedup mechanism.
4. Write `invoice_paid_for_unknown_subscription_is_noop` — confirms graceful handling.
5. Write `invoice_payment_failed_dispatches_admin_alert` — extends the IntegrationManager recording shim if needed.
6. Write `invoice_payment_failed_final_attempt_softens_copy` — asserts `is_final = true` is passed.
7. Write `invoice_payment_failed_for_unknown_subscription_is_noop`.
8. `cargo test --features test-utils --test stripe_webhook_test` — all new + existing tests green.
9. `cargo test --features test-utils` — full suite green.
