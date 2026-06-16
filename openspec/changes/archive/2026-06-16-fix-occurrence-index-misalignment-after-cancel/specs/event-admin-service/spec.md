# event-admin-service Specification Delta

## MODIFIED Requirements

### Requirement: Materializer respects per-occurrence exceptions

The recurring-event materializer (both initial materialization on series creation and the daily horizon-roll) SHALL consult `event_series_exceptions` for each `(series_id, occurrence_index)` pair it would otherwise create:

- If a `cancelled` exception exists → no `events` row is created for that index.
- If an `overridden` exception exists → the `events` row is created from the series template, then the `override_payload`'s non-null fields are applied on top.
- If no exception exists → the `events` row is created from the series template as before.

The materializer SHALL assign `occurrence_index` by the occurrence's **position in the recurrence stream** (counting every stream position, including those that resolve to a `cancelled` exception), consistently across initial materialization and every horizon-roll. Because `cancel_event_occurrence` hard-deletes the cancelled occurrence's `events` row (without lowering `series.materialized_through`), the horizon-roll SHALL NOT infer the next index from a bare aggregate over surviving rows that assumes a survivor sits at its nominal stream position — neither `MAX(occurrence_index)` (a cancelled *trailing* occurrence would shift all later indices down by one and collide a future occurrence with the past cancellation's exception) nor a count anchored on the earliest surviving row treated as index 1 (a cancelled *leading* occurrence would shift the inferred anchor forward and undercount the base index). The horizon-roll SHALL instead derive the base index from a surviving occurrence's own stored `occurrence_index` (its true stream position), keeping numbering aligned regardless of which occurrences were cancelled/deleted.

This guarantees that:
- A cancelled occurrence does NOT reappear on the next horizon-roll.
- An overridden occurrence's overrides do NOT get clobbered when materialization re-runs.
- Cancelling the highest-numbered materialized occurrence does NOT cause the next horizon-roll to skip a real future occurrence or misalign later indices.
- Cancelling the lowest-numbered (first) materialized occurrence does NOT cause the next horizon-roll to undercount the base index or collide a new occurrence with a surviving index.

#### Scenario: Cancelled occurrence stays cancelled across horizon-roll

- **WHEN** an occurrence is cancelled via `cancel_event_occurrence`, then the daily materializer runs (`now + 52 weeks` extends the horizon past the cancelled occurrence's date)
- **THEN** the materializer SHALL NOT recreate an `events` row for that occurrence index; the cancellation persists

#### Scenario: Overridden occurrence overrides survive series re-edit

- **WHEN** occurrence 7 has an `overridden` exception (location = "Room B"), then `update_series` is called with a cutoff before occurrence 7 (forcing re-materialization)
- **THEN** the `events` row for occurrence 7 SHALL be re-created with the series's updated template fields AND the override's location = "Room B" applied on top

#### Scenario: Cancelling the boundary occurrence does not shift later indices

- **WHEN** a series is materialized through occurrence N (the current horizon boundary), an admin cancels occurrence N (deleting its `events` row and leaving `materialized_through` unchanged), and the horizon-roll then materializes further occurrences
- **THEN** the occurrence at stream position N+1 SHALL be created (it SHALL NOT be skipped by colliding with occurrence N's `cancelled` exception), and the newly created rows SHALL carry `occurrence_index = N+1, N+2, …` matching their stream positions

#### Scenario: Cancelling the first occurrence does not shift later indices

- **WHEN** a series is materialized through occurrence N, an admin cancels occurrence 1 (deleting its `events` row so the re-derived `MIN(start_time)` anchor jumps to occurrence 2, and leaving `materialized_through` unchanged), and the horizon-roll then materializes further occurrences
- **THEN** the newly created rows SHALL carry `occurrence_index = N+1, N+2, …` matching their stream positions, and SHALL NOT be shifted down to collide with a surviving occurrence's index
