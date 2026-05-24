## Why

`src/web/portal/payments.rs` is 964 lines holding 17 handlers covering member payment views, the checkout flow (Stripe.js + saved-card charge), saved-card management (list, set-default, delete, auto-renew toggle), and receipts. Reviewing changes to the saved-card removal flow currently means scrolling through 800 lines of unrelated handlers.

The structure already has clean groupings — payment views (~5 handlers), payment flow pages (~3), checkout APIs (~2), saved cards (~5), receipts (~2). This makes it a clean candidate for the same split pattern applied in `a28` (admin/members handlers).

## What Changes

- Convert `src/web/portal/payments.rs` → `src/web/portal/payments/` module directory.
- Proposed submodule layout (from grep at spec time):
  - `mod.rs` — router + module declarations
  - `views.rs` — `payments_page`, `payments_list_api`, `payments_summary_api`, `dues_status_api`, `next_due_api`
  - `flow.rs` — `payment_success_page`, `payment_cancel_page`, `payment_new_page`
  - `checkout.rs` — `checkout_api`, `charge_saved_card_api`
  - `saved_cards.rs` — `payment_methods_page`, `update_auto_renew_api`, `saved_cards_html_api`, `delete_card_api`, `set_default_card_api`
  - `receipts.rs` — `receipts_page`, `receipt_page`

## Capabilities

### New Capabilities
- `portal-payments-handlers-layout`: portal payments handlers are organized into focused submodules, each ≤300 lines.

### Modified Capabilities
None.

## Impact

- **Code**: net-neutral.
- **Wire shape**: zero change — routes resolve to the same handlers from different files.
- **Tests**: no test changes needed; integration tests hit routes, not handler paths.
- **Risk**: low. Mechanical refactor mirroring `a28`. Risk areas: imports per submodule, visibility on handlers (`pub` or `pub(super)`), and the rate-limiter wiring on `checkout_api` and `charge_saved_card_api` (must continue to be invoked).
- **Dependency**: none. Independent of all other queued changes.
