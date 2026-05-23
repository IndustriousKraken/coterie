## 1. Create the module directory

- [ ] 1.1 Create `src/payments/webhook_dispatcher/` directory.
- [ ] 1.2 Create empty `mod.rs`, `payment_intent.rs`, `charge.rs`, `checkout.rs`, `invoice.rs`, `subscription.rs`.

## 2. Move struct + entry-point + test seams to mod.rs

- [ ] 2.1 Move `WebhookDispatcher` struct definition + `pub fn new(...)` to `mod.rs`.
- [ ] 2.2 Move `pub async fn handle_webhook(...)` (the event-type router, current line 65) to `mod.rs`. This function dispatches to the per-event handlers; after the split it will reference them via their new module paths.
- [ ] 2.3 Move the second `impl WebhookDispatcher` block (the `dispatch_*` test seams, current line 963+, conditionally compiled with `#[cfg(any(test, feature = "test-utils"))]`) to `mod.rs` verbatim.
- [ ] 2.4 Add `mod payment_intent; mod charge; mod checkout; mod invoice; mod subscription;` declarations at the top of `mod.rs`.

## 3. Move per-event handlers

- [ ] 3.1 `payment_intent.rs`: move `handle_payment_intent_succeeded` (line 410), `handle_failed_payment` (379), `handle_successful_payment` (197). All inside `impl WebhookDispatcher { ... }`.
- [ ] 3.2 `charge.rs`: move `handle_charge_refunded` (578).
- [ ] 3.3 `checkout.rs`: move `handle_expired_session` (361).
- [ ] 3.4 `invoice.rs`: move `handle_invoice_paid` (677), `handle_invoice_payment_failed` (796).
- [ ] 3.5 `subscription.rs`: move `handle_subscription_deleted` (879), `handle_subscription_updated` (933).

## 4. Visibility

- [ ] 4.1 Change each per-event handler from private (`async fn`) to `pub(super) async fn` so `mod.rs`'s `handle_webhook` router can call them.

## 5. Imports

- [ ] 5.1 Add the needed `use` statements at the top of each submodule. Start by copying the current full `use` block from the old `webhook_dispatcher.rs` and prune per-submodule.
- [ ] 5.2 `cargo build` will flag any missing imports; resolve.

## 6. Update self-calls (if any cross handlers)

- [ ] 6.1 Any handler that calls another handler via `self.other_handler(...)` continues to work without change (Rust resolves the method via the type). Verify no syntactic adjustments are needed — `cargo build` will catch any cases where module-path qualification is necessary.

## 7. Validation

- [ ] 7.1 `cargo build --features test-utils` — clean compile.
- [ ] 7.2 `cargo test --features test-utils` — all tests pass.
- [ ] 7.3 `cargo test --features test-utils -- stripe_webhook` — webhook-specific tests pass.
- [ ] 7.4 `cargo clippy --features test-utils -- --deny warnings` — clean.
- [ ] 7.5 `cargo fmt --check` — clean.
- [ ] 7.6 `wc -l src/payments/webhook_dispatcher/*.rs` — confirm no file exceeds 300 lines.
- [ ] 7.7 Delete the old `src/payments/webhook_dispatcher.rs`. Confirm `cargo build` still succeeds.
