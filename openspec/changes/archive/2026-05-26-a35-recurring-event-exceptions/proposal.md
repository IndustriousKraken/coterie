## Why

`RecurringEventService` and the `Recurrence` enum cover the bulk of recurring-event behavior: weekly/monthly-by-day/monthly-by-weekday patterns, 52-week rolling materialization, series-level edits affecting future occurrences. What's missing is **per-occurrence exception handling** — when one specific occurrence in a series needs to be cancelled or overridden without touching the rest of the series.

Real cases:
- A weekly meeting is cancelled for the December 25 holiday. The rest of the series continues unchanged. Admins shouldn't need to "end the series, start a new one starting January" or hand-edit the event row in a way that gets clobbered when the materializer rolls forward.
- A weekly meeting moves location for one week only (we're in a different building because of a one-time event). Same need: override one occurrence's fields without disturbing series-level data.

Today, neither is supported. The closest is `delete_series_occurrences_after`, which deletes all future occurrences from a date — useful for ending a series but not for a single skip.

## What Changes

- **New `event_series_exceptions` table**:
  - `series_id` (UUID) — FK to `event_series.id`.
  - `occurrence_index` (integer) — the 1-based position within the series being excepted.
  - `kind` (string) — `cancelled` or `overridden`.
  - `override_payload` (TEXT, nullable JSON) — for `overridden` kind, contains the field overrides (title, start_time, location, etc.); `NULL` for `cancelled`.
  - `created_at`, `created_by`, `audit_reason` (TEXT, optional).
  - Unique constraint on `(series_id, occurrence_index)`.
- **New service methods on `RecurringEventService`** (or a sibling `EventExceptionService` if cleaner):
  - `cancel_occurrence(series_id, occurrence_index, actor_id, reason)` — inserts a `cancelled` exception row + hard-deletes the corresponding `events` row.
  - `override_occurrence(series_id, occurrence_index, actor_id, overrides, reason)` — inserts an `overridden` exception row + updates the corresponding `events` row with the override values.
  - `restore_occurrence(series_id, occurrence_index, actor_id)` — removes the exception row + (for `cancelled`) re-materializes the single occurrence from the series rule.
- **Materializer SHALL consult the exceptions table**: when rolling the horizon forward (the daily job + on-create initial materialization), the materializer skips any `(series_id, occurrence_index)` already present in `event_series_exceptions`. This prevents a cancelled occurrence from reappearing on the next horizon-roll.
- **New admin handlers** (under `/portal/admin/events/series/:id/occurrences/:index/`):
  - `POST .../cancel` — calls `cancel_occurrence`.
  - `POST .../override` (multipart form) — calls `override_occurrence`.
  - `POST .../restore` — calls `restore_occurrence`.
- **UI on the event detail page** (existing recurring-event detail): "Cancel this occurrence" and "Edit just this occurrence" buttons next to each future occurrence in the series list. Cancelled occurrences show as struck-through with a "cancelled — restore" affordance.
- **Audit + integration events**: every exception action emits an audit row (`cancel_event_occurrence`, `override_event_occurrence`, `restore_event_occurrence`). Integration events: if the cancelled occurrence had a Discord-announced reminder pending, ideally the reminder is suppressed (out of scope for v1 — flagged in design.md).

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `event-admin-service` — gains per-occurrence cancel/override/restore methods alongside the existing series-level operations.
- `admin-events` — admin event detail page gains per-occurrence affordances.

## Impact

- **Code**: new migration for `event_series_exceptions` table; new repo methods (insert exception, list exceptions for series, delete exception); new service methods; new handlers + templates; materializer change to consult exceptions.
- **Wire shape**: three new `POST` endpoints. No existing routes change.
- **Tests**: cancel-an-occurrence, override-an-occurrence, materializer-respects-exception (after horizon-roll, cancelled occurrence does NOT reappear), restore-a-cancelled-occurrence, restore-an-override.
- **Risk**: low-medium. The materializer change is the riskiest piece — must not regress existing materialization behavior. Mitigation: integration tests cover both paths (with-exceptions and without-exceptions).
- **Dependency**: none in the active queue. Uses existing `event-admin-service` infrastructure.
