## 1. Create the module directory

- [ ] 1.1 Create `src/web/portal/payments/` directory.
- [ ] 1.2 Create empty `mod.rs`, `views.rs`, `flow.rs`, `checkout.rs`, `saved_cards.rs`, `receipts.rs`.

## 2. Move handlers per the inventory

- [ ] 2.1 `views.rs`: `payments_page` (127), `payments_list_api` (140), `payments_summary_api` (156), `dues_status_api` (176), `next_due_api` (188).
- [ ] 2.2 `flow.rs`: `payment_success_page` (200), `payment_cancel_page` (212), `payment_new_page` (224).
- [ ] 2.3 `checkout.rs`: `checkout_api` (283), `charge_saved_card_api` (332).
- [ ] 2.4 `saved_cards.rs`: `payment_methods_page` (504), `update_auto_renew_api` (556), `saved_cards_html_api` (638), `delete_card_api` (654), `set_default_card_api` (719).
- [ ] 2.5 `receipts.rs`: `receipts_page` (806), `receipt_page` (886).

## 3. Reconcile imports

- [ ] 3.1 For each new submodule, add the `use` statements its handlers need. Start by copying the current `payments.rs` `use` block and prune per-submodule.
- [ ] 3.2 `cargo build` will flag any missing imports; resolve.

## 4. Update mod.rs

- [ ] 4.1 Strip `mod.rs` to:
  - `use` block for whatever the router itself needs (Axum types, the State types it threads through)
  - `mod views; mod flow; mod checkout; mod saved_cards; mod receipts;`
  - The `routes()` function (or equivalent) wiring paths to handlers in their new modules.
- [ ] 4.2 Update handler references in `routes()` to use their new module paths (e.g., `views::payments_page`, `checkout::checkout_api`).

## 5. Rate-limiter wiring verification

- [ ] 5.1 Confirm the `money_limiter` layer is still applied to the routes for `checkout_api` and `charge_saved_card_api`. The wiring lives in `mod.rs`'s router builder (or whatever assembles the route tree); the split MUST NOT drop this layer.
- [ ] 5.2 If there's an existing integration test exercising the rate limit (look in `tests/` for "money_limiter" or "rate" matches), run it and confirm it still passes.

## 6. Visibility

- [ ] 6.1 For each handler, try `pub(super) async fn` first. Escalate to `pub async fn` only if `cargo build` errors.

## 7. Validation

- [ ] 7.1 `cargo build --features test-utils` — clean compile.
- [ ] 7.2 `cargo test --features test-utils` — all tests pass.
- [ ] 7.3 `cargo clippy --features test-utils -- --deny warnings` — clean.
- [ ] 7.4 `cargo fmt --check` — clean.
- [ ] 7.5 `wc -l src/web/portal/payments/*.rs` — confirm no file exceeds 300 lines.
- [ ] 7.6 Delete the old `src/web/portal/payments.rs`. Confirm `cargo build` still succeeds.
