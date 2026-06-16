# event-admin-service Specification Delta

## MODIFIED Requirements

### Requirement: Materializer respects per-occurrence exceptions

The recurring-event materializer (both initial materialization on series creation and the daily horizon-roll) SHALL consult `event_series_exceptions` for each `(series_id, occurrence_index)` pair it would otherwise create:

- If a `cancelled` exception exists → no `events` row is created for that index.
- If an `overridden` exception exists → the `events` row is created from the series template, then the `override_payload`'s non-null fields are applied on top.
- If no exception exists → the `events` row is created from the series template as before.

The materializer SHALL assign `occurrence_index` by the occurrence's **position in the recurrence stream** (counting every stream position, including those that resolve to a `cancelled` exception), consistently across initial materialization and every horizon-roll. The horizon-roll SHALL NOT derive the next index from `MAX(occurrence_index)` over surviving `events` rows, because cancelling an occurrence hard-deletes its `events` row (without lowering `series.materialized_through`); deriving from surviving rows would let a cancelled boundary occurrence shift all later indices down by one and collide a future occurrence with the past cancellation's exception.

This guarantees that:
- A cancelled occurrence does NOT reappear on the next horizon-roll.
- An overridden occurrence's overrides do NOT get clobbered when materialization re-runs.
- Cancelling the highest-numbered materialized occurrence does NOT cause the next horizon-roll to skip a real future occurrence or misalign later indices.

#### Scenario: Cancelled occurrence stays cancelled across horizon-roll

- **WHEN** an occurrence is cancelled via `cancel_event_occurrence`, then the daily materializer runs (`now + 52 weeks` extends the horizon past the cancelled occurrence's date)
- **THEN** the materializer SHALL NOT recreate an `events` row for that occurrence index; the cancellation persists

#### Scenario: Overridden occurrence overrides survive series re-edit

- **WHEN** occurrence 7 has an `overridden` exception (location = "Room B"), then `update_series` is called with a cutoff before occurrence 7 (forcing re-materialization)
- **THEN** the `events` row for occurrence 7 SHALL be re-created with the series's updated template fields AND the override's location = "Room B" applied on top

#### Scenario: Cancelling the boundary occurrence does not shift later indices

- **WHEN** a series is materialized through occurrence N (the current horizon boundary), an admin cancels occurrence N (deleting its `events` row and leaving `materialized_through` unchanged), and the horizon-roll then materializes further occurrences
- **THEN** the occurrence at stream position N+1 SHALL be created (it SHALL NOT be skipped by colliding with occurrence N's `cancelled` exception), and the newly created rows SHALL carry `occurrence_index = N+1, N+2, …` matching their stream positions
