## ADDED Requirements

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
