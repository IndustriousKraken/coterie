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

