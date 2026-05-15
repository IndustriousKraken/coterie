## Why

`src/service/billing_service.rs` is a 150-line shell whose only job is to forward 12 method calls to one of three sub-services (`auto_renew::AutoRenew`, `notifications::Notifications`, `expiration::Expiration`). Every method on the facade has a doc-prefix that names the sub-service it belongs to:

```rust
// ---- Auto-renew lifecycle + charge runner ---
pub async fn enable_auto_renew(...) -> Result<()> {
    self.auto_renew.enable_auto_renew(...).await
}

// ---- Notifications ---
pub async fn notify_subscription_cancelled(...) -> Result<()> {
    self.notifications.notify_subscription_cancelled(...).await
}
```

Two costs:

1. **Per-method maintenance tax**. Adding a new method to (say) `AutoRenew` requires a parallel forward on `BillingService`. Today the section comments inside the facade *already group methods by sub-service* — the structure is begging to be exposed.
2. **Hidden composition**. A reader following `billing_service.run_billing_cycle()` jumps to a one-line forwarder before getting to the real implementation. That extra hop adds nothing.

The original split (the doc comment on `BillingService` notes "splitting the original 1300-line `BillingService` along these lines means each sub-module has a single concern") was the right call. The facade is the part that's working against that win.

The fix is small and mechanical: keep `BillingService` as a *container* with three public fields, drop the delegation methods, and let callers go through the field. The structural insight ("auto-renew vs notifications vs expiration") becomes legible at the call site instead of buried in a comment.

## What Changes

- **`BillingService` becomes a struct with three public fields**:
  ```rust
  pub struct BillingService {
      pub auto_renew:    auto_renew::AutoRenew,
      pub notifications: notifications::Notifications,
      pub expiration:    expiration::Expiration,
  }
  ```
- **Sub-service types become `pub`**. Today `AutoRenew`, `Notifications`, `Expiration` are private to the `billing_service` module. They become `pub` so external callers can hold typed references (e.g., `&BillingService.auto_renew`). Their constructors stay `pub`.
- **All 12 forwarder methods on `BillingService` are removed**. `BillingService::new` stays — it constructs the three sub-services and the container.
- **Callers migrate to field access**:
  | Before                                                | After                                                              |
  |-------------------------------------------------------|--------------------------------------------------------------------|
  | `billing_service.run_billing_cycle()`                 | `billing_service.auto_renew.run_billing_cycle()`                   |
  | `billing_service.enable_auto_renew(...)`              | `billing_service.auto_renew.enable_auto_renew(...)`                |
  | `billing_service.disable_auto_renew(...)`             | `billing_service.auto_renew.disable_auto_renew(...)`               |
  | `billing_service.reschedule_after_payment(...)`       | `billing_service.auto_renew.reschedule_after_payment(...)`         |
  | `billing_service.extend_member_dues_by_slug(...)`     | `billing_service.auto_renew.extend_member_dues_by_slug(...)`       |
  | `billing_service.migrate_to_coterie_managed(...)`     | `billing_service.auto_renew.migrate_to_coterie_managed(...)`       |
  | `billing_service.bulk_migrate_stripe_subscriptions()` | `billing_service.auto_renew.bulk_migrate_stripe_subscriptions()`   |
  | `billing_service.notify_subscription_cancelled(...)`  | `billing_service.notifications.notify_subscription_cancelled(...)` |
  | `billing_service.notify_subscription_payment_failed(...)` | `billing_service.notifications.notify_subscription_payment_failed(...)` |
  | `billing_service.send_dues_reminders()`               | `billing_service.notifications.send_dues_reminders()`              |
  | `billing_service.check_expired_members()`             | `billing_service.expiration.check_expired_members()`               |
- **Re-exports stay**. The `pub use auto_renew::BulkMigrationSummary;` line in `billing_service.rs` keeps working.
- **Out of scope**: renaming methods (e.g., `auto_renew.enable_auto_renew` → `auto_renew.enable`). The suffix becomes redundant at some new call sites, but renaming is a judgment call that's separable from the structural change. This proposal preserves every method name exactly.
- **Out of scope**: renaming the `AppState` field (still `billing_service`, not `billing`).

## Capabilities

### New Capabilities

(None — internal restructure of an existing capability. No new spec file.)

### Modified Capabilities
- `recurring-billing`: adds an internal-structure requirement that `BillingService` is a flat container with three public sub-service fields, not a delegating facade. Externally-observable behavior (the runner's tick, dunning, dues-reminder cadence) is unchanged. The delta documents the access shape so a future contributor can't reintroduce delegation methods.

## Impact

- **Code**:
  - **Removed**: ~80 lines of delegation methods on `BillingService` in `src/service/billing_service.rs`.
  - **Modified**: visibility on `AutoRenew`, `Notifications`, `Expiration` flips from module-private to `pub`. Sub-module declarations in `billing_service.rs` may need `pub mod` instead of `mod`.
  - **Modified call sites**: ~15 method-call lines across 7 files (`src/jobs/billing_runner.rs`, `src/api/handlers/payments.rs`, `src/web/portal/admin/billing.rs`, `src/web/portal/payments.rs`, `src/payments/webhook_dispatcher.rs`, plus internal references in tests).
  - **Net**: ~60–80 lines removed; the file `billing_service.rs` shrinks from 150 to ~70 lines (constructor + struct).
- **Wire shape**: zero change. Same wire endpoints, same scheduling cadence, same audit rows, same integration events.
- **Tests**: test fixtures that today call facade methods need to update access paths. Existing tests assert behavior, not call structure, so the test bodies stay the same modulo the path rewrite. No new tests required by this change — the structural assertion is enforced by the spec delta.
- **Risk**: low. The change is purely mechanical (pattern: dot-access path lengthens by one segment). The compiler catches every missed migration. No behavior change.
- **Trade-off accepted**: call sites become slightly more verbose (`auto_renew.run_billing_cycle()` adds 11 chars over `run_billing_cycle()`). The verbosity is the *value* — a reader instantly knows which sub-service the call belongs to without grepping for `// ---- Auto-renew` section headers.
