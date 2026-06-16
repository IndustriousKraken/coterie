# scheduled-payments Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Scheduled-payment lifecycle has explicit states

A scheduled payment SHALL move through a finite set of states: `pending`, `processing`, `completed`, `failed`, `cancelled`. The normal path is `pending` → `processing` → `completed` (charge succeeded) or `failed` (charge failed after retries are exhausted). The terminal states are `completed`, `failed`, and `cancelled`; a row in a terminal state SHALL NOT be picked up by the runner.

A transient charge failure that has NOT yet exhausted the configured retry count MAY return the row from `processing` back to `pending` so a later runner pass retries it — this is the only transition back to `pending`. A row that has reached the terminal `failed` state SHALL NOT spontaneously revive.

#### Scenario: Pending row can be cancelled

- **WHEN** an admin or the system cancels a pending scheduled payment
- **THEN** the row SHALL transition to `cancelled` and SHALL NOT be picked up by the runner

#### Scenario: Failed row does not auto-revive

- **WHEN** a scheduled payment reaches the terminal `failed` state (retries exhausted)
- **THEN** it SHALL NOT spontaneously transition back to `pending`; a new scheduled-payment row SHALL be created if a retry is desired

#### Scenario: Transient failure before retries are exhausted returns to pending

- **WHEN** a charge attempt fails and the retry count has NOT yet reached the configured maximum
- **THEN** the row SHALL return from `processing` to `pending` (with the failure reason recorded) so a later runner pass retries it

### Requirement: Captured amount and currency are immutable on the row

Each scheduled-payment row SHALL record the amount and currency at the time of scheduling. Subsequent membership-type changes SHALL NOT mutate existing rows.

#### Scenario: Amount remains stable after type change

- **WHEN** an admin changes the dues amount on a membership type
- **THEN** existing pending scheduled-payment rows for that type SHALL retain their captured amount; new rows SHALL pick up the new amount

### Requirement: Pending→Processing transition is an atomic compare-and-swap

Claiming a scheduled payment for processing SHALL be an atomic compare-and-swap: the transition from `pending` to `processing` SHALL be performed by a single guarded statement (`UPDATE scheduled_payments SET status = 'processing', … WHERE id = ? AND status = 'pending'`) whose `rows_affected()` determines whether the caller won the claim. `process_scheduled_payment` SHALL NOT use a separate read-then-unconditional-write (which permits two concurrent callers to both pass a `status == pending` check and both proceed).

A caller that loses the claim (the guarded update affects zero rows) SHALL NOT charge the card, SHALL NOT create a `payments` row, and SHALL NOT extend the member's dues; it returns without side effects. This prevents a concurrency window in which two callers each mint a distinct `payment_id` for the same scheduled payment and each extend dues — the per-`payment_id` idempotency of dues extension does not protect against distinct `payment_id`s for one charge.

#### Scenario: Only one concurrent caller claims a pending scheduled payment

- **WHEN** two callers attempt to process the same `pending` scheduled payment concurrently
- **THEN** exactly one guarded `claim_for_processing` update SHALL affect one row (the winner), the other SHALL affect zero rows (the loser), and only the winner SHALL charge the card and extend dues

#### Scenario: Lost claim performs no side effects

- **WHEN** `process_scheduled_payment` is invoked for a scheduled payment whose status is already `processing` (already claimed by another worker)
- **THEN** the call SHALL return successfully without charging the card, without creating a new `payments` row, and without changing the member's `dues_paid_until`

#### Scenario: A non-pending scheduled payment is not reprocessed

- **WHEN** `process_scheduled_payment` is invoked for a scheduled payment whose status is `completed`, `failed`, or `cancelled`
- **THEN** the atomic claim SHALL affect zero rows and the call SHALL perform no charge and no dues extension

