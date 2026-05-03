## ADDED Requirements

### Requirement: Scheduled-payment lifecycle has explicit states

A scheduled payment SHALL move through a finite set of states: `pending`, `attempted`, `succeeded`, `failed`, `cancelled`. State transitions SHALL be linear (e.g., `pending` → `attempted` → `succeeded` or `failed`); a `cancelled` row SHALL be terminal.

#### Scenario: Pending row can be cancelled

- **WHEN** an admin or the system cancels a pending scheduled payment
- **THEN** the row SHALL transition to `cancelled` and SHALL NOT be picked up by the runner

#### Scenario: Failed row does not auto-revive

- **WHEN** a scheduled payment moves to `failed`
- **THEN** it SHALL NOT spontaneously transition back to `pending`; a new scheduled-payment row SHALL be created if a retry is desired

### Requirement: Captured amount and currency are immutable on the row

Each scheduled-payment row SHALL record the amount and currency at the time of scheduling. Subsequent membership-type changes SHALL NOT mutate existing rows.

#### Scenario: Amount remains stable after type change

- **WHEN** an admin changes the dues amount on a membership type
- **THEN** existing pending scheduled-payment rows for that type SHALL retain their captured amount; new rows SHALL pick up the new amount
