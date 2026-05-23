## ADDED Requirements

### Requirement: Portal payments handlers are organized into focused submodules

`src/web/portal/payments.rs` SHALL be converted to a module directory at `src/web/portal/payments/` with the following submodules:

- `mod.rs` — router + module declarations
- `views.rs` — `payments_page`, `payments_list_api`, `payments_summary_api`, `dues_status_api`, `next_due_api`
- `flow.rs` — `payment_success_page`, `payment_cancel_page`, `payment_new_page`
- `checkout.rs` — `checkout_api`, `charge_saved_card_api`
- `saved_cards.rs` — `payment_methods_page`, `update_auto_renew_api`, `saved_cards_html_api`, `delete_card_api`, `set_default_card_api`
- `receipts.rs` — `receipts_page`, `receipt_page`

Each handler's visibility SHALL be the narrowest that satisfies the router's needs (`pub(super)` preferred where it works; `pub` otherwise).

#### Scenario: No submodule exceeds 300 lines

- **WHEN** the post-split files in `src/web/portal/payments/` are line-counted
- **THEN** every file SHALL be ≤300 lines

#### Scenario: All routes still resolve to the same handlers

- **WHEN** integration tests that hit portal payment routes are run
- **THEN** they SHALL pass without modification; URL → handler resolution is unchanged

#### Scenario: Rate limiter remains wired on money-moving endpoints

- **WHEN** the router for `/portal/api/payments/checkout` and `/portal/api/payments/charge-saved` is inspected after the split
- **THEN** both routes SHALL still apply the `money_limiter` layer as required by the `rate-limiting` capability; the limiter wiring SHALL NOT regress as a side-effect of the split
