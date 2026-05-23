## Context

`src/web/portal/payments.rs` accumulated 964 lines. Function inventory (line numbers as of spec writing):

| Line | Function | Concern |
|------|----------|---------|
| 127  | `payments_page` | views |
| 140  | `payments_list_api` | views |
| 156  | `payments_summary_api` | views |
| 176  | `dues_status_api` | views |
| 188  | `next_due_api` | views |
| 200  | `payment_success_page` | flow |
| 212  | `payment_cancel_page` | flow |
| 224  | `payment_new_page` | flow |
| 283  | `checkout_api` | checkout (money-moving, rate-limited) |
| 332  | `charge_saved_card_api` | checkout (money-moving, rate-limited) |
| 504  | `payment_methods_page` | saved cards |
| 556  | `update_auto_renew_api` | saved cards |
| 638  | `saved_cards_html_api` | saved cards |
| 654  | `delete_card_api` | saved cards |
| 719  | `set_default_card_api` | saved cards |
| 806  | `receipts_page` | receipts |
| 886  | `receipt_page` | receipts |

The two checkout handlers (`checkout_api`, `charge_saved_card_api`) are the biggest individual concerns at ~50 and ~170 lines respectively. The saved-card group totals ~300 lines.

This follows the same pattern as `a28` (admin/members split). Same considerations apply.

## Goals / Non-Goals

**Goals:**
- Each submodule ≤300 lines.
- Router (`mod.rs`) stays slim — module declarations + route wiring.
- Per-concern files are easy to find: "where does delete-card live?" → `saved_cards.rs`.
- Rate-limiter wiring for money-moving endpoints (checkout, charge-saved) remains intact — these endpoints are listed in the `rate-limiting` capability spec.

**Non-Goals:**
- Changing handler behavior, signatures, or routes.
- Refactoring `charge_saved_card_api` (~170 lines) further — it's a long but cohesive function.
- Touching the saved-card repository or Stripe gateway code.

## Decisions

### D1. Submodule layout

```
src/web/portal/payments/
├── mod.rs           — router + module declarations
├── views.rs         — payments_page, payments_list_api, payments_summary_api, dues_status_api, next_due_api
├── flow.rs          — payment_success_page, payment_cancel_page, payment_new_page
├── checkout.rs      — checkout_api, charge_saved_card_api
├── saved_cards.rs   — payment_methods_page, update_auto_renew_api, saved_cards_html_api, delete_card_api, set_default_card_api
└── receipts.rs      — receipts_page, receipt_page
```

Estimated sizes after split:
- `views.rs`: ~100 lines
- `flow.rs`: ~80 lines
- `checkout.rs`: ~220 lines (the biggest — includes the long `charge_saved_card_api`)
- `saved_cards.rs`: ~300 lines (right at the threshold)
- `receipts.rs`: ~150 lines
- `mod.rs`: ~50 lines

If `saved_cards.rs` overshoots 300, split further: keep `payment_methods_page` + `saved_cards_html_api` in `saved_cards/list.rs`, `delete_card_api` + `set_default_card_api` + `update_auto_renew_api` in `saved_cards/manage.rs`. Don't pre-split.

### D2. Handler visibility

Same pattern as `a28`: use `pub(super)` where the router in `mod.rs` is the only caller. Fall back to `pub` if visibility errors surface.

### D3. Rate-limiter wiring preserved

The `rate-limiting` spec lists `POST /portal/api/payments/checkout` and `POST /portal/api/payments/charge-saved` as `money_limiter` callers. After the split, the router in `mod.rs` MUST still apply the `money_limiter` layer to these two routes. Verify post-split that the `tower::ServiceBuilder` / `axum::middleware::from_fn_with_state` chain is intact.

### D4. Tests

`portal/payments.rs` itself has no inline tests. Integration tests live in `tests/`. Nothing relocates.

## Risks / Trade-offs

- **Risk**: rate-limiter layer accidentally dropped during the split. → Mitigation: explicit task to verify the limiter wiring; tests covering checkout flow (if any) will fail if rate limiting is broken.
- **Risk**: a handler that was implicitly visible inside the file is no longer accessible from `mod.rs`'s router. → Mitigation: `pub(super)` or `pub` per `cargo build` feedback.
- **Trade-off**: 6 files instead of 1. Consistent with the codebase pattern.

## Migration Plan

Same shape as `a28`:

1. Create the submodule files.
2. Move each handler group + its private helpers (none of significance in this file).
3. Reconcile imports per submodule.
4. Strip `mod.rs` to router + declarations.
5. Verify rate-limiter wiring on checkout endpoints.
6. `cargo build`, `cargo test`, `cargo clippy --deny warnings`, `cargo fmt --check`.
7. Delete old `src/web/portal/payments.rs`.
