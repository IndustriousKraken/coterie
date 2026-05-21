## 1. Familiarize with existing harness

- [ ] 1.1 Read `tests/stripe_webhook_test.rs` end-to-end. Identify the `build_harness()` function, the `Harness` struct, the helper fns (`insert_member`, `member_dues_paid_until`, `payment_status`, etc.), and the existing test pattern.
- [ ] 1.2 Locate the `WebhookDispatcher::handle_webhook` entry point and the `dispatch_*` test seams (`dispatch_payment_intent_succeeded`, etc.) in `src/payments/webhook_dispatcher.rs`. The invoice handlers have similar dispatch seams; confirm naming and use them.

## 2. Add IntegrationManager test-recording shim if needed

- [ ] 2.1 Check whether the harness's `IntegrationManager` currently records dispatched events for inspection. If yes, the existing recording is sufficient — skip 2.2.
- [ ] 2.2 If not, extend `IntegrationManager` with a `#[cfg(any(test, feature = "test-utils"))]`-gated `dispatched_events: Arc<Mutex<Vec<IntegrationEvent>>>` field. Push to it inside `handle_event` (in the same gated block) AFTER the normal dispatch. Tests inspect via a helper `events_recorded(&manager) -> Vec<IntegrationEvent>`.
- [ ] 2.3 Alternative: register a test-only `RecordingIntegration` impl that just appends to a shared `Vec`. Either approach works. Pick whichever fits the existing test infrastructure with the least churn.

## 3. handle_invoice_paid tests

- [ ] 3.1 Add `invoice_paid_extends_dues_for_stripe_subscription_member`. Setup: seed a `StripeSubscription`-mode member with `dues_paid_until = today + 30 days` (relative anchor per the `a14` rule), `stripe_subscription_id = sub_test_123`. Build a `stripe::Invoice` via direct construction or JSON-deserialize with `subscription = "sub_test_123"`, `status = "paid"`, `period_end` corresponding to "today + 60 days". Call `dispatcher.dispatch_invoice_paid(invoice)` (or whatever the seam is). Assert: `dues_paid_until` advanced (use `member_dues_paid_until` helper); a Payment row exists for the member with the invoice's payment_intent ID and status Completed.
- [ ] 3.2 Add `invoice_paid_idempotency`. Same setup. Dispatch the same invoice event twice. Assert: `dues_paid_until` advanced ONCE (compute the expected value before the second dispatch and assert it didn't change). Assert: exactly one Payment row exists for that invoice.
- [ ] 3.3 Add `invoice_paid_for_unknown_subscription_is_noop`. Dispatch an invoice event with `subscription = "sub_NEVER_HEARD_OF"`. Assert: no Payment rows created; the member's dues_paid_until unchanged from setup.

## 4. handle_invoice_payment_failed tests

- [ ] 4.1 Add `invoice_payment_failed_dispatches_admin_alert`. Setup: seed a `StripeSubscription`-mode member. Build an invoice event with `status = "open"` and `attempt_count = 1`, `next_payment_attempt = Some(<future-timestamp>)`. Dispatch. Assert: an `IntegrationEvent::AdminAlert` is in the IntegrationManager's recorded-events list, with subject containing "Stripe subscription charge failed". Assert: `dues_paid_until` unchanged. Assert: `billing_mode` unchanged.
- [ ] 4.2 Add `invoice_payment_failed_final_attempt_softens_copy`. Same setup but with `next_payment_attempt = None`. Dispatch. Assert: the AdminAlert subject contains "(final)". This proves the `is_final = true` branch was taken inside `notify_subscription_payment_failed`.
- [ ] 4.3 Add `invoice_payment_failed_for_unknown_subscription_is_noop`. Dispatch with `subscription = "sub_NEVER_HEARD_OF"`. Assert: no AdminAlert was recorded; no DB changes.

## 5. Validate

- [ ] 5.1 `cargo test --features test-utils --test stripe_webhook_test` — all tests in the file pass (existing + new).
- [ ] 5.2 `cargo test --features test-utils` — full suite passes.
- [ ] 5.3 If any test surfaces a bug in `handle_invoice_paid` or `handle_invoice_payment_failed`, document it clearly in the commit message and either fix in the same PR (if minor) or spec a separate change (if larger).
