# recurring-billing Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Background billing runner schedules and charges dues

The system SHALL run a background billing job (`billing_runner`) that periodically:

1. Identifies members whose `dues_paid_until` is approaching the configured renewal lead time.
2. For members opted in to auto-renew with a saved default card, attempts to charge the dues amount.
3. Records each attempt as a payment row through the payment service so audit and integration-event side-effects fire.
4. Updates `dues_paid_until` on success.

#### Scenario: Member with auto-renew opt-in is charged before expiry

- **WHEN** the runner finds an Active auto-renew member whose `dues_paid_until` is within the lead window AND who has a default saved card
- **THEN** the runner SHALL attempt the charge and update `dues_paid_until` on success

#### Scenario: Member without saved card is not charged

- **WHEN** an auto-renew member has no default saved card
- **THEN** the runner SHALL NOT attempt a charge and SHALL leave the member to the natural expiry/dunning flow

### Requirement: Charge failures enter dunning, not silent retry

When a charge attempt fails, the runner SHALL record the failure on the payment row, emit the relevant integration event, and surface the failure on the admin billing dashboard. The runner SHALL NOT silently retry within the same run.

#### Scenario: Failed charge surfaces on dashboard

- **WHEN** a charge fails (card decline, network error)
- **THEN** the failure SHALL appear in `/portal/admin/billing/dashboard` recent-failures view

#### Scenario: Failed charge does not advance dues_paid_until

- **WHEN** a charge fails
- **THEN** `dues_paid_until` SHALL NOT change; the member SHALL move to Expired at the natural transition

### Requirement: Runner is idempotent over a single tick

The runner SHALL ensure that running the same tick twice does not double-charge a member. Idempotency SHALL be enforced via the payment service plus repository-level conflict handling, not by relying on the scheduler firing exactly once.

#### Scenario: Re-run within the lead window does not re-charge a successful member

- **WHEN** the runner re-runs a few minutes after a successful charge
- **THEN** the previously-charged member SHALL be excluded from the candidate set

### Requirement: BillingService is a flat container, not a delegating facade

`BillingService` SHALL be a struct holding three public fields — one per sub-service — and SHALL NOT carry delegation methods that forward to those sub-services. The fields SHALL be:

- `pub auto_renew: AutoRenew`
- `pub notifications: Notifications`
- `pub expiration: Expiration`

Callers SHALL reach sub-service methods via direct field access (e.g., `billing_service.auto_renew.run_billing_cycle()`). Adding a new method to a sub-service SHALL NOT require any change to `BillingService` itself.

#### Scenario: New auto-renew method needs no facade update

- **WHEN** a contributor adds a new method to `AutoRenew` (e.g., a one-shot reconcile)
- **THEN** the method SHALL be reachable from external callers as `billing_service.auto_renew.<method>(...)` immediately, without adding a forwarder to `BillingService`

#### Scenario: BillingService impl block is constructor-only

- **WHEN** a contributor inspects `impl BillingService { ... }`
- **THEN** the only method SHALL be `new(...)` (the constructor that wires the three sub-services); there SHALL be no `pub async fn enable_auto_renew`, `pub async fn run_billing_cycle`, or any other forwarding method

### Requirement: Sub-service types and modules are public

`AutoRenew`, `Notifications`, and `Expiration` (and their containing modules `auto_renew`, `notifications`, `expiration` under `src/service/billing_service/`) SHALL be `pub` so that external callers can hold typed references via `BillingService`'s public fields. The sub-services SHALL continue to be constructed exclusively inside `BillingService::new`; no external code SHALL construct them directly.

#### Scenario: Sub-service paths resolve from external callers

- **WHEN** a caller writes `let svc: &auto_renew::AutoRenew = &state.billing_service.auto_renew;`
- **THEN** the path SHALL resolve and the borrow SHALL compile

#### Scenario: Sub-services are still constructed only inside BillingService::new

- **WHEN** any code outside `BillingService::new` attempts to construct an `AutoRenew`, `Notifications`, or `Expiration`
- **THEN** convention prohibits this; the sub-services SHALL remain owned-by-value inside the parent `BillingService`

### Requirement: Method names on sub-services are unchanged by the facade flattening

The flattening of `BillingService` SHALL NOT rename any method. `auto_renew.enable_auto_renew(...)`, `auto_renew.run_billing_cycle(...)`, `notifications.send_dues_reminders(...)`, `expiration.check_expired_members(...)`, and the rest SHALL keep the names they had before the change. Renaming methods (e.g., `enable_auto_renew` → `enable`) is a separate concern.

#### Scenario: Method names survive the change byte-for-byte

- **WHEN** a contributor diffs the post-change sub-service files against the pre-change ones
- **THEN** method signatures (name + parameter list + return type) SHALL be unchanged; only call-site access paths in the rest of the codebase SHALL change

