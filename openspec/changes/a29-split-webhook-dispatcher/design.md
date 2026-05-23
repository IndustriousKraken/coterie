## Context

`src/payments/webhook_dispatcher.rs` accumulated 1007 lines over the course of building up Stripe integration. The file has a clear two-`impl` shape:

- Lines 44–963: first `impl WebhookDispatcher` block. Holds `new()`, `handle_webhook` (the event-type router), and all nine per-event handlers.
- Lines 963–1007: second `impl` block, conditionally compiled with `#[cfg(any(test, feature = "test-utils"))]`. Holds the `dispatch_*` test seams that bypass signature verification and feed canned events into the handlers.

Function inventory:

| Line | Function | Lines | Event family |
|------|----------|-------|--------------|
| 65   | `handle_webhook` | 132 | router (kept in mod.rs) |
| 197  | `handle_successful_payment` | 164 | payment_intent (older/general path) |
| 361  | `handle_expired_session` | 18 | checkout.session.expired |
| 379  | `handle_failed_payment` | 31 | payment_intent.payment_failed |
| 410  | `handle_payment_intent_succeeded` | 168 | payment_intent.succeeded |
| 578  | `handle_charge_refunded` | 99 | charge.refunded |
| 677  | `handle_invoice_paid` | 119 | invoice.paid |
| 796  | `handle_invoice_payment_failed` | 83 | invoice.payment_failed |
| 879  | `handle_subscription_deleted` | 54 | customer.subscription.deleted |
| 933  | `handle_subscription_updated` | 30 | customer.subscription.updated |

The biggest handler is `handle_payment_intent_succeeded` at ~168 lines. Reasonable size for one function.

## Goals / Non-Goals

**Goals:**
- Each submodule ≤300 lines.
- The event router (`handle_webhook`) and `WebhookDispatcher` struct stay in `mod.rs`, since they're the entry point.
- Per-event handlers are grouped by Stripe event-type prefix (`payment_intent.*`, `invoice.*`, `customer.subscription.*`, etc.).
- Test seams (the `dispatch_*` functions, line 963+) stay in `mod.rs` since they reference all handler functions and benefit from being centralized.

**Non-Goals:**
- Changing any handler's behavior or signature.
- Splitting `handle_payment_intent_succeeded` further (it's a long function but cohesive — that's a separate concern).
- Touching `tests/stripe_webhook_test.rs` (also flagged in the arch pass but out of scope here).

## Decisions

### D1. Submodule layout

```
src/payments/webhook_dispatcher/
├── mod.rs              — struct, new(), handle_webhook router, dispatch_* test seams
├── payment_intent.rs   — handle_payment_intent_succeeded, handle_failed_payment, handle_successful_payment
├── charge.rs           — handle_charge_refunded
├── checkout.rs         — handle_expired_session
├── invoice.rs          — handle_invoice_paid, handle_invoice_payment_failed
└── subscription.rs     — handle_subscription_deleted, handle_subscription_updated
```

### D2. Test seams stay in mod.rs

The `dispatch_*` functions are thin wrappers that exist for the test suite to bypass Stripe signature verification. They reference every per-event handler. Moving them to sibling files would force them to live in each submodule, fragmenting the test-seam surface. Keep them in `mod.rs`.

If `mod.rs`'s total size becomes an issue, the test seams could live in a `test_seams.rs` submodule declared as `#[cfg(any(test, feature = "test-utils"))] mod test_seams;`. Don't pre-split — only if needed.

### D3. Handler visibility

Per-event handlers are currently `async fn` (private). Moving them to sibling files requires them to be at least `pub(super)` so `mod.rs`'s `handle_webhook` router can call them as `payment_intent::handle_payment_intent_succeeded(self, event)` etc.

Use `pub(super)` — the narrowest visibility that satisfies the cross-submodule call pattern.

### D4. Self-method-call ergonomics

Currently the handlers call each other implicitly via `self.handle_failed_payment(...)`. After the split, the calls become `self.handle_failed_payment(...)` still — because they're all `impl WebhookDispatcher` methods, Rust resolves them via the type. No syntactic change at call sites, even across submodule boundaries.

Wait — that requires all handlers to still be in `impl WebhookDispatcher` blocks. They are, by the design above. So no call-site changes are needed.

## Risks / Trade-offs

- **Risk**: webhook dispatch is critical-path; a missed import or visibility error could cause silent compilation failure → fixed by `cargo build` (errors hard, doesn't silently miss).
- **Risk**: regressions in event handling. → Mitigation: the existing test suite is the safety net. After the split, `cargo test stripe_webhook` should pass with the same assertions.
- **Trade-off**: 6 files instead of 1. Acceptable; each file is now grep-able by event type.

## Migration Plan

Single PR.

1. Create `src/payments/webhook_dispatcher/` directory + the 5 submodules + `mod.rs`.
2. Move struct + `new()` + `handle_webhook` + the second `impl` block (`dispatch_*` test seams) to `mod.rs`.
3. Move each per-event handler to its target submodule, in an `impl WebhookDispatcher { ... }` block.
4. Reconcile imports per submodule.
5. Update visibility to `pub(super)` on the per-event handlers so `mod.rs` can call them.
6. `cargo build`, `cargo test --features test-utils`, `cargo clippy --deny warnings` — all clean.
7. Delete old `src/payments/webhook_dispatcher.rs`.
