## Context

`BillingService` currently looks like this:

```rust
pub struct BillingService {
    auto_renew:    auto_renew::AutoRenew,
    notifications: notifications::Notifications,
    expiration:    expiration::Expiration,
}

impl BillingService {
    pub fn new(...) -> Self { ... }

    // ---- Auto-renew lifecycle + charge runner ----
    pub async fn migrate_to_coterie_managed(...) -> Result<bool> {
        self.auto_renew.migrate_to_coterie_managed(...).await
    }
    pub async fn enable_auto_renew(...) -> Result<()> {
        self.auto_renew.enable_auto_renew(...).await
    }
    // … 5 more auto-renew forwarders

    // ---- Notifications ----
    pub async fn notify_subscription_cancelled(...) -> Result<()> {
        self.notifications.notify_subscription_cancelled(...).await
    }
    // … 2 more notification forwarders

    // ---- Expiration sweep ----
    pub async fn check_expired_members(...) -> Result<u32> {
        self.expiration.check_expired_members().await
    }
}
```

The facade exists as a transitional artifact from the larger `BillingService` split (per the file's doc comment: "Splitting the original 1300-line `BillingService` along these lines means each sub-module has a single concern"). The split is right; the facade is the part that's working against it.

Each forwarder is a one-liner with no validation, no logging, no transformation. They exist purely so callers can write `billing_service.run_billing_cycle()` instead of `billing_service.auto_renew.run_billing_cycle()`. That's a thin save that's not worth ~80 lines of dead-weight code and the ongoing tax of keeping the facade in sync as sub-services evolve.

## Goals / Non-Goals

**Goals:**
- Eliminate the 12 delegation methods on `BillingService`.
- Expose the three sub-services as public fields so callers can reach them by direct field access.
- Make the structural grouping ("this is auto-renew work" / "this is a notification" / "this is an expiration sweep") legible at every call site.
- Preserve byte-equivalent runtime behavior — the runner's tick, dunning, and dues-reminder pipelines all still execute the same calls in the same order.

**Non-Goals:**
- Renaming methods to drop redundant suffixes (e.g., `auto_renew.enable_auto_renew` → `auto_renew.enable`). The redundancy is real but the rename is a separate judgment call; bundling it would balloon the PR diff and pull in a debate that's separable.
- Renaming the `AppState` field (`billing_service` → `billing`). Same logic — out of scope.
- Splitting `BillingService` further or merging sub-services. The current three-way split is the reason this change is even worth doing.
- Touching the sub-services' constructors or internal state.
- Changing the spec at the level of "what auto-renew does" or "what dunning looks like." The spec delta is purely structural.

## Decisions

### D1. Sub-service types become `pub`, not `pub(crate)`

The fields are `pub`, so the types they hold must be reachable by external callers. Considered: `pub(crate)` so the types stay confined to the crate. Rejected — `BillingService` is itself `pub` and reachable from `AppState`, so callers in any module already use it. There's no useful encapsulation gained by `pub(crate)` here, and `pub` is the simpler, more honest declaration.

### D2. Sub-modules become `pub mod`, not `mod`

In `src/service/billing_service.rs`:

```rust
pub mod auto_renew;
pub mod notifications;
pub mod expiration;
```

…so `crate::service::billing_service::auto_renew::AutoRenew` is a reachable path. Today they're `mod` (private). External callers don't typically construct sub-services directly — they reach them through the field — but making the modules public removes the artificial barrier. The sub-services stay constructed once inside `BillingService::new`; nothing external builds them.

### D3. The container struct keeps a `new()` constructor

```rust
impl BillingService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        scheduled_payment_repo: Arc<dyn ScheduledPaymentRepository>,
        payment_repo: Arc<dyn PaymentRepository>,
        // ... same params as today
    ) -> Self {
        Self {
            auto_renew:    auto_renew::AutoRenew::new(...),
            notifications: notifications::Notifications::new(...),
            expiration:    expiration::Expiration::new(...),
        }
    }
}
```

The constructor is the only `impl BillingService` block after this change; it carries the dependency-wiring that was previously split across the sub-service constructors plus the facade. Identical to today.

### D4. Public fields, not getter methods

```rust
pub struct BillingService {
    pub auto_renew:    auto_renew::AutoRenew,
    pub notifications: notifications::Notifications,
    pub expiration:    expiration::Expiration,
}
```

Considered: keep the fields private and add `pub fn auto_renew(&self) -> &auto_renew::AutoRenew`. Rejected — that's a delegation method dressed up as a getter; it has the same maintenance tax as the methods we're removing. Public fields are the honest shape.

The objection to public fields is usually "it lets callers mutate state without the struct's permission." That doesn't apply here:
- Callers hold `Arc<BillingService>`, so they can't get `&mut`.
- The sub-services themselves manage their own internal state (Arc'd repos, Arc'd integration manager). Field access reads the sub-service value (which is an owned struct holding Arcs) and calls its methods.

### D5. Method names are preserved

`auto_renew.enable_auto_renew(...)` reads with a redundant suffix at the new call site. Tempting to rename to `auto_renew.enable(...)`. Out of scope per non-goal. The change is purely structural; renaming becomes a follow-up if the redundancy bothers anyone in practice.

The same applies to `notifications.notify_subscription_cancelled(...)` (could be `notifications.subscription_cancelled(...)`), `expiration.check_expired_members(...)` (could be `expiration.sweep()`), etc.

### D6. Constructor parameter order stays the same

`BillingService::new` keeps its 11-parameter signature. The clippy `too_many_arguments` allow stays. The constructor wires the same Arcs into the same sub-service constructors. No fields rearrange.

### D7. `BulkMigrationSummary` re-export is preserved

```rust
pub use auto_renew::BulkMigrationSummary;
```

Stays. Callers writing `use crate::service::billing_service::BulkMigrationSummary;` keep working.

After the change, the type is also reachable as `crate::service::billing_service::auto_renew::BulkMigrationSummary` — preferable for new code, but not required.

### D8. Callers update one segment of access path; nothing else

Every caller migration is a single-character edit pattern:

```rust
// Before
billing_service.run_billing_cycle().await

// After
billing_service.auto_renew.run_billing_cycle().await
```

A grep-and-rewrite per method yields the migration. The compiler catches stragglers.

The rewrite is mechanical because the receiver type stays the same: today `billing_service` is `&BillingService` (or `Arc<BillingService>`), tomorrow it's the same. The new call adds one field deref before the method call.

### D9. The `WebhookDispatcher` keeps taking `&BillingService`

`webhook_dispatcher.rs` passes `&BillingService` through its handler chain. That signature is unchanged — every internal call inside the dispatcher rewrites from `billing_service.foo(...)` to `billing_service.auto_renew.foo(...)` (or `.notifications.`). The dispatcher's contract with the rest of the application is preserved.

### D10. The change ships in one PR

Single mechanical refactor with compiler-enforced correctness. Land in one PR; deploy normally. `git revert` is the rollback.

## Risks / Trade-offs

- **Risk**: a caller is missed because it lives in a test, an example binary, or a feature-gated path. → **Mitigation**: `cargo build --all-targets --features test-utils` exercises tests + binaries; CI catches any straggler.
- **Risk**: a consumer of `BillingService` that we don't own (none today, but) needs the facade methods. → **Mitigation**: the codebase is single-crate; there are no external consumers. Spec'd as an internal change.
- **Trade-off**: call-site verbosity grows. `auto_renew.run_billing_cycle()` is longer than `run_billing_cycle()`. The longer form is more self-documenting; that trade is the value of the change.
- **Trade-off**: a future contributor who wants to add a method to multiple sub-services (e.g., a hypothetical "pause-everything" admin action) has nowhere obvious to put it. → **Acceptable**: that scenario doesn't exist today; if it appears, add a method on `BillingService` itself at that time. The container struct can grow methods without becoming a delegating facade.
- **Trade-off**: documentation comments on the facade methods (e.g., the rationale for `BillingService::extend_member_dues_by_slug` not rolling back the payment row on failure) live on the *forwarder* today. They need to migrate to the sub-service method (or already exist there — verify in implementation).

## Migration Plan

Single PR; no deployment ceremony.

1. In `src/service/billing_service.rs`: change `mod auto_renew;` → `pub mod auto_renew;` (and likewise for the other two). Make `pub struct AutoRenew { ... }` (and likewise) inside each sub-module file.
2. Convert `BillingService` to a struct with three `pub` fields. Drop every delegation method. Keep `new` and the `pub use` re-export.
3. Migrate callers in `src/jobs/billing_runner.rs`, `src/api/handlers/payments.rs`, `src/web/portal/admin/billing.rs`, `src/web/portal/payments.rs`, `src/payments/webhook_dispatcher.rs`, plus any tests that call facade methods.
4. `cargo build --all-targets --features test-utils`.
5. `cargo test --features test-utils`.
6. Eyeball: `wc -l src/service/billing_service.rs` should be ~70 (down from 150).
