## ADDED Requirements

### Requirement: Admin event detail page exposes per-occurrence exception controls

The recurring-event detail page in the admin portal SHALL show, for each future occurrence in the series:

- A "Cancel this occurrence" button. Clicking it opens an optional-reason prompt and POSTs to `/portal/admin/events/series/:id/occurrences/:index/cancel`. On success, the occurrence row is replaced with a struck-through "Cancelled — [reason] — restore" line.
- An "Edit just this occurrence" button. Clicking it opens a modal/form containing only the overridable fields (per `event-admin-service`'s `OccurrenceOverride` shape). Submit POSTs to `/portal/admin/events/series/:id/occurrences/:index/override`.

Past occurrences (start_time < now) SHALL NOT show the cancel/override controls — exceptions only apply to the future.

A "restore" link SHALL appear on any cancelled OR overridden occurrence (past or future) so the admin can undo an exception.

The three new POST routes SHALL require admin authentication + CSRF (inherits the portal middleware tree).

#### Scenario: Cancel a future occurrence from the admin UI

- **WHEN** an admin on the event-series detail page clicks "Cancel this occurrence" on occurrence 5 (a future occurrence) and confirms with reason "holiday"
- **THEN** the browser issues `POST /portal/admin/events/series/:id/occurrences/5/cancel`; on success the occurrence row updates in place to show "Cancelled — holiday — restore"

#### Scenario: Restore a cancelled occurrence

- **WHEN** an admin clicks "restore" on a previously-cancelled occurrence
- **THEN** the browser issues `POST /portal/admin/events/series/:id/occurrences/5/restore`; on success the occurrence row updates to show the re-materialized occurrence's normal display

#### Scenario: Past occurrences hide the cancel control

- **WHEN** an admin views the series detail page and the series has both past and future occurrences
- **THEN** "Cancel this occurrence" and "Edit just this occurrence" controls SHALL appear only on future occurrences; the past occurrences SHALL show only the "restore" link IF they have an existing exception
