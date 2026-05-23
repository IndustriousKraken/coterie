## Why

`src/payments/webhook_dispatcher.rs` is 1007 lines holding the Stripe webhook router (`handle_webhook`) plus nine per-event handlers covering payment_intent, charge, invoice, subscription, and checkout-session events. This is critical-path payment code — webhook handlers are how Coterie learns about every dues renewal, refund, failed payment, and subscription cancellation. Reviewing changes to one event handler currently means scrolling through 1000 lines of unrelated event handlers.

Splitting by event-family makes each file ~100–250 lines, each focused on one Stripe event-type prefix.

## What Changes

- Convert `src/payments/webhook_dispatcher.rs` → `src/payments/webhook_dispatcher/` module directory.
- Submodule layout (from grep at spec time):
  - `mod.rs` — `WebhookDispatcher` struct, `new()`, `handle_webhook` (event-type router), and the `dispatch_*` test seams (the second `impl` block at line 963)
  - `payment_intent.rs` — `handle_payment_intent_succeeded` (410), `handle_failed_payment` (379), `handle_successful_payment` (197) [the older/general handler]
  - `charge.rs` — `handle_charge_refunded` (578)
  - `checkout.rs` — `handle_expired_session` (361, checkout-session.expired)
  - `invoice.rs` — `handle_invoice_paid` (677), `handle_invoice_payment_failed` (796)
  - `subscription.rs` — `handle_subscription_deleted` (879), `handle_subscription_updated` (933)
- All handler methods stay in `impl WebhookDispatcher` blocks across the new files (multiple `impl` blocks for the same type, no API change).
- Tests in `tests/stripe_webhook_test.rs` continue to use the `dispatch_*` test seams; their import path doesn't change.

## Capabilities

### New Capabilities
- `webhook-dispatcher-layout`: `WebhookDispatcher` is organized as a module directory with per-event-family submodules, each ≤300 lines.

### Modified Capabilities
None.

## Impact

- **Code**: net-neutral.
- **Wire shape**: zero change. Same router entrypoint (`handle_webhook`), same dispatched events.
- **Tests**: `tests/stripe_webhook_test.rs` (1119 lines, separately flagged by the arch pass but out of scope here) continues to call `dispatch_*` seams unchanged.
- **Risk**: low-medium. The webhook dispatcher is critical-path; a regression here could mean missed payments. Mitigation: the existing test suite is comprehensive — `tests/stripe_webhook_test.rs` exercises every dispatched event type via the `dispatch_*` seams.
- **Dependency**: none.
