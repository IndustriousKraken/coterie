## MODIFIED Requirements

### Requirement: Validation at the service boundary

`record_manual` SHALL validate at entry: `amount_cents >= 0`,
`amount_cents <= MAX_PAYMENT_CENTS`, member exists, and donation-campaign id
exists when supplied. These checks defend against forged JSON / stale forms
even though the UI normally produces valid input.

#### Scenario: Negative amount rejected

- **WHEN** `record_manual` receives `amount_cents = -100`
- **THEN** it SHALL return `BadRequest` AND the `payments` table SHALL
  remain empty (no row was persisted before the guard fired)

#### Scenario: Amount over cap rejected

- **WHEN** `record_manual` receives an amount exceeding `MAX_PAYMENT_CENTS`
- **THEN** it SHALL return `BadRequest` whose message names the cap
  in whole dollars

#### Scenario: Unknown member rejected

- **WHEN** `record_manual` receives a `member_id` not present in `members`
- **THEN** it SHALL return `BadRequest` whose message includes the
  unknown id

#### Scenario: Donation with stale campaign id rejected

- **WHEN** `record_manual` receives `PaymentKind::Donation { campaign_id:
  Some(stale_id) }` where the campaign no longer exists
- **THEN** the call SHALL return `BadRequest` AND the `payments` table
  SHALL contain no row for this attempt (no orphan donation row is created)

### Requirement: PaymentService::record_manual emits the audit-log entry

`record_manual` SHALL emit an audit-log entry via `audit_service.log` after a
successful repo write, using a centralized `audit_action(method, kind)`
mapping that produces the action string. The mapping SHALL be:

- `(Waived, _)` → `"waive_dues"`
- `(_, Membership)` → `"manual_payment"`
- `(_, Donation { .. })` → `"manual_donation"`
- `(_, Other)` → `"manual_other"`

Centralization SHALL prevent the four sites that previously duplicated this
from drifting.

#### Scenario: Cash dues payment audits as manual_payment

- **WHEN** `record_manual` records a `(PaymentMethod::Cash,
  PaymentKind::Membership)` payment
- **THEN** the emitted audit row SHALL have `action = "manual_payment"`

#### Scenario: Waived dues audits as waive_dues

- **WHEN** `record_manual` records a `(PaymentMethod::Waived,
  PaymentKind::Membership)` payment
- **THEN** the emitted audit row SHALL have `action = "waive_dues"`

#### Scenario: Cash donation audits as manual_donation

- **WHEN** `record_manual` records a `(PaymentMethod::Cash,
  PaymentKind::Donation { .. })` payment
- **THEN** the emitted audit row SHALL have `action = "manual_donation"`
