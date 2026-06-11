## Context

`RecurringEventService` currently models a series with two responsibilities:
1. Initial materialization: at series creation, generate concrete `events` rows up to the 52-week horizon.
2. Horizon roll-forward (daily job): when `now + 52 weeks > series.materialized_through`, materialize newly-uncovered occurrences.

Series-level edits (e.g., changing the title for all future meetings) update the template + propagate to occurrences after a cutoff via `delete_series_occurrences_after` + re-materialize. This works because the unit of "edit" is the entire future tail of the series.

What doesn't work: editing or skipping a single occurrence. If an admin manually edits the row in `events` for occurrence 5 (say, changing its location), the next horizon-roll might re-materialize it — or worse, the series-level edit might delete-and-recreate it, clobbering the manual change. There's no place to record "this occurrence is special."

This change introduces an exception table that names per-occurrence overrides and cancellations explicitly, and teaches the materializer to consult it.

## Goals / Non-Goals

**Goals:**
- Cancel a single occurrence without affecting the rest of the series. Restorable.
- Override a single occurrence's fields (title, start/end time, location, etc.) without affecting the rest of the series. Restorable to the series template.
- The materializer's horizon-roll respects exceptions: cancelled occurrences don't reappear; overridden occurrences don't get their overrides clobbered.
- Series-level edits (change-future) respect exceptions: cancelled occurrences stay cancelled, overrides stay applied to overridden occurrences (the override "wins").

**Non-Goals:**
- Adding new occurrences to a series outside the recurrence rule. If admins need ad-hoc additions, those should be one-off events, not series additions.
- Bulk exception management (e.g., "cancel all occurrences in December"). v1 is one-at-a-time. Bulk operations are a future iteration.
- Suppressing Discord reminders / event-reminder emails for cancelled occurrences. Flagged as a follow-up — see Risks. v1 cancellation is data-layer only; if a reminder is already scheduled, it fires.
- Changing the `Recurrence` enum or supported patterns. The enum stays as-is.

## Decisions

### D1. Exception table schema

```sql
CREATE TABLE event_series_exceptions (
    series_id TEXT NOT NULL REFERENCES event_series(id) ON DELETE CASCADE,
    occurrence_index INTEGER NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('cancelled', 'overridden')),
    override_payload TEXT,  -- JSON, NULL for cancelled
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    created_by TEXT NOT NULL REFERENCES members(id),
    audit_reason TEXT,
    PRIMARY KEY (series_id, occurrence_index)
);

CREATE INDEX idx_exceptions_series ON event_series_exceptions(series_id);
```

The primary key is `(series_id, occurrence_index)` — there can be at most one exception per occurrence. Restoring an exception deletes the row.

### D2. The override_payload JSON shape

A subset of the `Event` fields that can be overridden:

```json
{
  "title": "Optional<string>",
  "description": "Optional<string>",
  "start_time": "Optional<RFC3339>",
  "end_time": "Optional<RFC3339>",
  "location": "Optional<string>",
  "max_attendees": "Optional<int>",
  "rsvp_required": "Optional<bool>",
  "image_url": "Optional<string>"
}
```

`null` field means "use the series template value." Field deserializes via `serde(default)` on an `OccurrenceOverride` struct.

`event_type` and `visibility` are NOT overridable in v1 — they're series-level concerns. If a future need surfaces, add them then.

### D3. Materializer change

`RecurringEventService::materialize_horizon` (or whichever method handles the daily roll-forward) consults `event_series_exceptions` for each series being processed. For each new `(series_id, occurrence_index)` it would create:

- If an exception row with `kind = 'cancelled'` exists → skip creation entirely.
- If an exception row with `kind = 'overridden'` exists → create the `events` row, then apply the `override_payload` fields on top of the series template's fields.
- Otherwise → create the `events` row from the series template as today.

Same logic applies on initial materialization (though exceptions on a brand-new series are unusual — possible if an admin creates a series + immediately cancels one occurrence + waits for re-materialization in a test scenario).

### D4. Series-level edit interaction

When `update_series` (or whichever method changes the series template for future occurrences) runs:

1. It identifies the cutoff (e.g., occurrence_index = N forward).
2. `delete_series_occurrences_after` removes future occurrence rows.
3. The materializer re-creates them from the updated template.
4. **Exception rows persist across this process.** When the materializer re-creates the rows, it consults exceptions per D3 — cancelled occurrences stay cancelled, overridden occurrences re-apply their overrides on top of the new template.

The semantic: exceptions are stable across series-level edits. An override that said "this Tuesday moves to Wednesday at 7pm" continues to mean that, even if the series's default time changes to 6pm.

### D5. Service API

```rust
impl EventAdminService {
    pub async fn cancel_event_occurrence(
        &self,
        actor_id: Uuid,
        series_id: Uuid,
        occurrence_index: i32,
        reason: Option<String>,
    ) -> Result<()>;

    pub async fn override_event_occurrence(
        &self,
        actor_id: Uuid,
        series_id: Uuid,
        occurrence_index: i32,
        overrides: OccurrenceOverride,
        reason: Option<String>,
    ) -> Result<Event>;

    pub async fn restore_event_occurrence(
        &self,
        actor_id: Uuid,
        series_id: Uuid,
        occurrence_index: i32,
    ) -> Result<Option<Event>>;  // Some if restored from cancelled (re-materialized), None if restored from override (existing row updated)
}
```

Each method:
- Validates the series exists and the occurrence_index is plausible (≥1, ≤ series.materialized_through's occurrence_index).
- Emits an audit row (`cancel_event_occurrence`, `override_event_occurrence`, `restore_event_occurrence`) per the audit-logging capability contract.
- For `cancel`: insert exception row + DELETE the `events` row (idempotent — if already deleted/cancelled, no-op + log).
- For `override`: insert exception row + UPDATE the `events` row with override values.
- For `restore`: DELETE exception row + re-materialize/reset the `events` row.

### D6. Admin UI

On the recurring-event detail page (already exists per `admin-events` capability), the occurrence list gains two affordances per future occurrence:

- "Cancel this occurrence" — HTMX POST to `.../cancel`, prompts for optional reason, replaces the row with a struck-through version showing "Cancelled — [reason] — restore".
- "Edit just this occurrence" — HTMX GET opens a modal/form with only the overridable fields; submit POSTs to `.../override`.

Past occurrences (before `now`) get no exception affordances — exceptions only make sense for the future.

The "restore" link on a cancelled-or-overridden occurrence is always available (allows undo even on a past occurrence, just in case).

### D7. Tests

- Unit: `OccurrenceOverride` deserialization (all-fields, none-fields, mixed).
- Repo: insert exception, query exceptions for series, delete exception.
- Service: each of the three methods + their audit emission.
- Materializer: a series with a `cancelled` exception at index 5 — horizon roll doesn't recreate index 5.
- Materializer: a series with an `overridden` exception at index 7 — horizon roll creates index 7 with overrides applied.
- Series edit + exceptions: edit-future, exceptions survive the re-materialization.

## Risks / Trade-offs

- **Risk**: per-occurrence overrides accumulate. If an admin overrides 30 occurrences in a year, the exception table grows. → Mitigation: it grows linearly, with `(series_id, occurrence_index)` indexed; query cost is negligible. Cleanup-on-series-delete via the `ON DELETE CASCADE`.
- **Risk**: cancelled occurrences should also suppress event-reminder emails (per the `event-reminders` capability). v1 doesn't do this. → Mitigation: flag as a follow-up. If reminders are dispatched as a scheduled job that reads the `events` table, cancelled occurrences are already absent from the table — the reminder for that occurrence simply doesn't get sent. So this may be a non-issue depending on how `event-reminders` is wired. Verify during implementation.
- **Risk**: race between materializer and a cancel that just happened. If the materializer is mid-run and creates the row, then `cancel_event_occurrence` runs, then the materializer commits — could result in the row existing AND an exception row existing. → Mitigation: the cancel service method must DELETE the row even if it just got materialized, after inserting the exception. The cancel is the source of truth.
- **Trade-off**: overrides ARE persisted as JSON, which means schema evolution requires care. v1 is small (8 fields). Bigger overrides would warrant first-class columns.

## Migration Plan

Single PR.

1. New SQL migration: `event_series_exceptions` table.
2. New domain types: `OccurrenceException`, `OccurrenceOverride`.
3. New repo methods on `EventSeriesRepository`.
4. New service methods on `EventAdminService` (or a sibling).
5. Materializer change: consult exceptions before creating each occurrence.
6. New admin handlers + HTMX templates.
7. New audit-action strings registered.
8. Integration tests for each path.
9. `cargo test`, `cargo clippy --deny warnings`, `cargo fmt --check`.
