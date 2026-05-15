## 1. Make sub-modules and sub-service types public

- [ ] 1.1 In `src/service/billing_service.rs`, change `mod auto_renew;` → `pub mod auto_renew;` (and the same for `notifications` and `expiration`).
- [ ] 1.2 In `src/service/billing_service/auto_renew.rs`, change `pub struct AutoRenew { ... }` (likely already `pub`; confirm and adjust) so it remains reachable. Confirm the constructor `pub fn new(...)` is `pub`.
- [ ] 1.3 In `src/service/billing_service/notifications.rs`, do the same for `Notifications`.
- [ ] 1.4 In `src/service/billing_service/expiration.rs`, do the same for `Expiration`.

## 2. Flatten `BillingService` itself

- [ ] 2.1 In `src/service/billing_service.rs`, change the `BillingService` struct fields from private to `pub`:
  ```rust
  pub struct BillingService {
      pub auto_renew:    auto_renew::AutoRenew,
      pub notifications: notifications::Notifications,
      pub expiration:    expiration::Expiration,
  }
  ```
- [ ] 2.2 Keep `BillingService::new(...)` exactly as today — same parameter list, same body wiring the three sub-services.
- [ ] 2.3 Delete every delegation method from `impl BillingService { ... }`:
  - `migrate_to_coterie_managed`
  - `bulk_migrate_stripe_subscriptions`
  - `enable_auto_renew`
  - `reschedule_after_payment`
  - `disable_auto_renew`
  - `run_billing_cycle`
  - `extend_member_dues_by_slug`
  - `notify_subscription_cancelled`
  - `notify_subscription_payment_failed`
  - `send_dues_reminders`
  - `check_expired_members`
- [ ] 2.4 Keep `pub use auto_renew::BulkMigrationSummary;` re-export.
- [ ] 2.5 Migrate any documentation comments that today live on the deleted forwarders (e.g., the rationale for `extend_member_dues_by_slug`'s failure semantics) onto the corresponding sub-service method, IF the sub-service method doesn't already carry the same comment.

## 3. Migrate callers

- [ ] 3.1 `src/jobs/billing_runner.rs`: rewrite `billing_service.run_billing_cycle()` → `billing_service.auto_renew.run_billing_cycle()`. Same for `check_expired_members` (→ `expiration.`) and `send_dues_reminders` (→ `notifications.`).
- [ ] 3.2 `src/api/handlers/payments.rs`: rewrite `state.billing_service.migrate_to_coterie_managed(...)` → `state.billing_service.auto_renew.migrate_to_coterie_managed(...)`.
- [ ] 3.3 `src/web/portal/admin/billing.rs`: rewrite `state.billing_service.bulk_migrate_stripe_subscriptions()` → `state.billing_service.auto_renew.bulk_migrate_stripe_subscriptions()`.
- [ ] 3.4 `src/web/portal/payments.rs`: rewrite `billing_service.extend_member_dues_by_slug(...)`, `enable_auto_renew(...)`, `reschedule_after_payment(...)` to route through the `auto_renew` field.
- [ ] 3.5 `src/payments/webhook_dispatcher.rs`: rewrite the seven internal call sites that today call facade methods (`extend_member_dues_by_slug`, `reschedule_after_payment`, `notify_subscription_payment_failed`, `notify_subscription_cancelled`) to route through the appropriate sub-service field.
- [ ] 3.6 `src/web/portal/admin/members.rs`: this file passes `&billing_service` into `PaymentService::record_manual` — verify the signature still matches and no further changes are needed.

## 4. Migrate tests

- [ ] 4.1 `cargo build --all-targets --features test-utils` to surface every test that calls a facade method.
- [ ] 4.2 For each compile error, rewrite the call to route through the appropriate sub-service field. Test bodies and assertions are unchanged otherwise.

## 5. Verify

- [ ] 5.1 `cargo build --all-targets --features test-utils` — clean.
- [ ] 5.2 `cargo test --features test-utils` — full suite passes.
- [ ] 5.3 Eyeball: `wc -l src/service/billing_service.rs` — expected target ~70 (down from ~150).
- [ ] 5.4 Confirm no `impl BillingService` block other than the constructor exists in `billing_service.rs`. Grep `pub async fn` inside that file should show zero results outside the (constructor-only) impl block.
- [ ] 5.5 Confirm the `WebhookDispatcher` signature `&BillingService` is unchanged at the boundary — only its internal method calls were rewritten.

## 6. Spec sync

- [ ] 6.1 Confirm the change's delta spec (`openspec/changes/flatten-billing-service-facade/specs/recurring-billing/spec.md`) matches the implemented behavior.
- [ ] 6.2 At archive time (`opsx:archive`), the new requirements about the flat container structure, public sub-services, and unchanged method names merge into `openspec/specs/recurring-billing/spec.md`.
