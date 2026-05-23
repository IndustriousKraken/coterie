## 1. Create the module directory

- [x] 1.1 Create `src/payments/webhook_dispatcher/` directory.
- [x] 1.2 Create empty `mod.rs`, `payment_intent.rs`, `charge.rs`, `checkout.rs`, `invoice.rs`, `subscription.rs`.

## 2. Move struct + entry-point + test seams to mod.rs

- [x] 2.1 Move `WebhookDispatcher` struct definition + `pub fn new(...)` to `mod.rs`.
- [x] 2.2 Move `pub async fn handle_webhook(...)` (the event-type router, current line 65) to `mod.rs`. This function dispatches to the per-event handlers; after the split it will reference them via their new module paths.
- [x] 2.3 Move the second `impl WebhookDispatcher` block (the `dispatch_*` test seams, current line 963+, conditionally compiled with `#[cfg(any(test, feature = "test-utils"))]`) to `mod.rs` verbatim.
- [x] 2.4 Add `mod payment_intent; mod charge; mod checkout; mod invoice; mod subscription;` declarations at the top of `mod.rs`.

## 3. Move per-event handlers

- [x] 3.1 `payment_intent.rs`: move `handle_payment_intent_succeeded` (line 410), `handle_failed_payment` (379). `handle_successful_payment` (197) was placed in `checkout.rs` instead — it handles the `checkout.session.completed` event (parameter type `CheckoutSession`) so it sits alongside `handle_expired_session`. This grouping keeps `payment_intent.rs` under the 300-line scenario limit; the spec's listing of it under `payment_intent.rs` conflicted with that scenario.
- [x] 3.2 `charge.rs`: move `handle_charge_refunded` (578).
- [x] 3.3 `checkout.rs`: move `handle_expired_session` (361) and `handle_successful_payment` (197, see 3.1).
- [x] 3.4 `invoice.rs`: move `handle_invoice_paid` (677), `handle_invoice_payment_failed` (796).
- [x] 3.5 `subscription.rs`: move `handle_subscription_deleted` (879), `handle_subscription_updated` (933).

## 4. Visibility

- [x] 4.1 Change each per-event handler from private (`async fn`) to `pub(super) async fn` so `mod.rs`'s `handle_webhook` router can call them.

## 5. Imports

- [x] 5.1 Add the needed `use` statements at the top of each submodule. Start by copying the current full `use` block from the old `webhook_dispatcher.rs` and prune per-submodule.
- [x] 5.2 `cargo build` will flag any missing imports; resolve.

## 6. Update self-calls (if any cross handlers)

- [x] 6.1 Any handler that calls another handler via `self.other_handler(...)` continues to work without change (Rust resolves the method via the type). Verify no syntactic adjustments are needed — `cargo build` will catch any cases where module-path qualification is necessary.

## 7. Validation

- [x] 7.1 `cargo build --features test-utils` — clean compile.
- [x] 7.2 `cargo test --features test-utils` — all tests pass. (The 6 `member_template_snapshots` failures are pre-existing golden HTML drift unrelated to webhook_dispatcher; verified by re-running on baseline.)
- [x] 7.3 `cargo test --features test-utils -- stripe_webhook` — webhook-specific tests pass (13/13).
- [x] 7.4 `cargo clippy --features test-utils -- --deny warnings` — no new errors introduced by this change (baseline already had 66 unrelated errors; mine has 65).
- [x] 7.5 `cargo fmt --check` — clean for `src/payments/webhook_dispatcher/**`. (Baseline codebase has ~2141 fmt diffs unrelated to this change.)
- [x] 7.6 `wc -l src/payments/webhook_dispatcher/*.rs` — confirmed no file exceeds 300 lines (max: mod.rs at 259).
- [x] 7.7 Delete the old `src/payments/webhook_dispatcher.rs`. Confirm `cargo build` still succeeds.
