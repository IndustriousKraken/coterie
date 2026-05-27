## ADDED Requirements

### Requirement: EventAdminService surfaces typed errors for invalid inputs

`EventAdminService` mutation methods SHALL return typed `AppError` variants — not panics, not opaque `Internal` errors — for these operator-reachable failure inputs: `update_one` called with a missing `event_id` SHALL return `AppError::NotFound("Event not found")` AND SHALL short-circuit BEFORE writing the audit row (a 404 on the wire MUST NOT produce a phantom `update_event` audit entry); `cancel_event_occurrence`, `override_event_occurrence`, and `restore_event_occurrence` called with `occurrence_index < 1` SHALL return `AppError::BadRequest("occurrence_index must be >= 1")`; and each of those three methods called with a missing `series_id` SHALL return `AppError::NotFound` with a message containing the series id and "not found".

#### Scenario: update_one against missing event_id returns 4xx and writes no audit

- **WHEN** a handler invokes `svc.update_one(actor, missing_event_id, input)` against an `event_id` that does not exist
- **THEN** the method SHALL return `Err(AppError::NotFound(msg))` where `msg.contains("Event not found")`, AND `audit_logs` SHALL contain zero new rows with `action = "update_event"` and `entity_id = missing_event_id`

#### Scenario: Zero-index on occurrence-exception methods returns 4xx

- **WHEN** a handler invokes any of `cancel_event_occurrence(_, _, 0, _)`, `override_event_occurrence(_, _, 0, _, _)`, or `restore_event_occurrence(_, _, 0)`
- **THEN** each method SHALL return `Err(AppError::BadRequest(msg))` where `msg.contains("occurrence_index must be >= 1")`

#### Scenario: Unknown series_id on occurrence-exception methods returns 4xx

- **WHEN** a handler invokes any of the three occurrence-exception methods with a `series_id` that no `event_series` row matches
- **THEN** each method SHALL return `Err(AppError::NotFound(msg))` where `msg.contains("series")` and `msg.contains("not found")`

### Requirement: restore_event_occurrence SHALL audit the no-exception no-op

When `restore_event_occurrence(series_id, occurrence_index)` is invoked against a `(series, index)` pair that has NO `event_series_exceptions` row, the method SHALL return `Ok(None)` (no occurrence resurrection, no events row updated) AND SHALL still write an audit row with `action = "restore_event_occurrence"` and `entity_id = format!("{series_id}#{occurrence_index}")` so the operator's no-op click is still traceable in the audit log.

#### Scenario: Restore with no existing exception still audits

- **WHEN** an admin clicks "Restore" on an occurrence that was never cancelled or overridden (e.g., a UI race or a stale tab)
- **THEN** the service SHALL return `Ok(None)`, AND an `audit_logs` row with `action = "restore_event_occurrence"` and `entity_id = "{series}#{index}"` SHALL be present
