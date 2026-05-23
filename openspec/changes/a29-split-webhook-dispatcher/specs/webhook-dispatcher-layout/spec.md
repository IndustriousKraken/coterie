## ADDED Requirements

### Requirement: WebhookDispatcher is organized into per-event-family submodules

`src/payments/webhook_dispatcher.rs` SHALL be converted to a module directory at `src/payments/webhook_dispatcher/` with the following submodules:

- `mod.rs` — `WebhookDispatcher` struct, `new()`, `handle_webhook` (the event-type router), and the `dispatch_*` test seams (currently in the second `impl` block)
- `payment_intent.rs` — `handle_payment_intent_succeeded`, `handle_failed_payment`, `handle_successful_payment`
- `charge.rs` — `handle_charge_refunded`
- `checkout.rs` — `handle_expired_session`
- `invoice.rs` — `handle_invoice_paid`, `handle_invoice_payment_failed`
- `subscription.rs` — `handle_subscription_deleted`, `handle_subscription_updated`

All handler methods SHALL remain in `impl WebhookDispatcher` blocks (multiple `impl` blocks for the same type are valid Rust). Per-event handlers SHALL have visibility `pub(super)` so the router in `mod.rs` can call them.

#### Scenario: No submodule exceeds 300 lines

- **WHEN** the post-split files in `src/payments/webhook_dispatcher/` are line-counted
- **THEN** every file SHALL be ≤300 lines

#### Scenario: All webhook tests still pass

- **WHEN** `cargo test --features test-utils -- stripe_webhook` is run after the split
- **THEN** every test in `tests/stripe_webhook_test.rs` SHALL still pass with the same assertions; no test logic needs to change

#### Scenario: The dispatch_* test seams remain in mod.rs

- **WHEN** the `dispatch_payment_intent_succeeded`, `dispatch_charge_refunded`, `dispatch_subscription_deleted`, `dispatch_checkout_session_completed`, `dispatch_invoice_paid`, `dispatch_invoice_payment_failed` functions are located after the split
- **THEN** they SHALL live in `mod.rs` (or a single `test_seams.rs` submodule if `mod.rs` becomes too large), NOT scattered across the per-event submodules

#### Scenario: External callers compile unchanged

- **WHEN** code outside `src/payments/webhook_dispatcher/` that calls `WebhookDispatcher::new()` or `WebhookDispatcher::handle_webhook(...)` is built after the split
- **THEN** it SHALL compile without modification
