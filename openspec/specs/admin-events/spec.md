# admin-events Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Admin can create, view, update, and delete events

Admin event management SHALL be available at `/portal/admin/events` with the routes:
- `GET /portal/admin/events` — listing.
- `GET /portal/admin/events/new` and `POST /portal/admin/events/new` — create.
- `GET /portal/admin/events/:id` — detail.
- `POST /portal/admin/events/:id/update` — update.
- `POST /portal/admin/events/:id/delete` — delete.

Forms SHALL accept `multipart/form-data` to permit image uploads. CSRF MUST be the first multipart field so the top-level layer can validate before buffering large bodies.

#### Scenario: Event create with image succeeds

- **WHEN** an admin submits an event-create form with a JPEG image and `csrf_token` as the first field
- **THEN** the CSRF middleware SHALL validate the token, the handler SHALL persist the event via `event_repo`, then the handler SHALL call `audit_service.log` and dispatch `IntegrationEvent::EventPublished` (audit and integration emission are handler-owned for events)

#### Scenario: Multipart over the size cap is rejected with 400

- **WHEN** a multipart body exceeds 12MB
- **THEN** the CSRF middleware SHALL return a 400 "Request body too large"

### Requirement: Recurring events are managed as series

Events with a recurrence pattern SHALL be stored as a series with a generator that produces concrete event instances. The recurring-event service SHALL be the only entry point for materializing instances. Editing the series SHALL update future occurrences without rewriting historical ones.

#### Scenario: Editing series updates only future instances

- **WHEN** an admin edits the series description after some past occurrences exist
- **THEN** historical instances SHALL keep their original description; future instances SHALL adopt the edit

#### Scenario: Deleting series cancels future instances

- **WHEN** an admin deletes a series
- **THEN** future materialized instances SHALL be removed; past instances SHALL be retained for audit

### Requirement: Event-admin handlers route through EventAdminService

Admin event mutation handlers (`admin_create_event`, `admin_update_event`, `admin_delete_event`, plus the recurring-series variants) SHALL parse the wire shape (multipart form, path params, current user) and call `EventAdminService` for the actual mutation work. Handlers SHALL NOT call `event_repo.{create,update,delete}`, `audit_service.log`, or `integration_manager.handle_event` directly for these flows.

The wire shape (URLs, multipart bodies, HTMX response fragments) is unchanged.

#### Scenario: admin_create_event routes through the service

- **WHEN** an admin submits the new-event form
- **THEN** the handler SHALL build a `CreateEventInput` from the parsed multipart fields and call `EventAdminService::create(current_user.id, input)`; the side-effect chain runs inside the service

#### Scenario: Series-vs-single decision lives in the service

- **WHEN** the new-event form includes `repeat_kind != "none"`
- **THEN** the handler SHALL include the parsed recurrence rule on the `CreateEventInput`; the service decides whether to call `RecurringEventService::materialize_series(...)` vs. a single insert based on the input's `recurrence` field

### Requirement: Tests of the recurring-event materializer use anchors relative to runtime

Tests asserting on `RecurringEventService::create_series_with_initial_materialization` (or the materializer's output more generally) SHALL compute their input anchors and `until_date` values relative to `Utc::now()` at runtime. Fixed-calendar timestamps SHALL NOT be used as test inputs to the materializer.

This rule applies BOTH to standalone test files under `tests/` AND to inline `#[cfg(test)] mod tests` blocks inside `src/` files (such as `src/service/event_admin_service.rs`). Wherever a test exercises the materializer, the inputs SHALL be runtime-relative.

The reason: the materializer's horizon is `now + 12 months`. A fixed-calendar anchor drifts further into the past as wall-clock time advances, changing the gap between the anchor and the horizon. Tests that assert occurrence counts (with any tolerance) inevitably break as the gap widens. Tests that constrain via a fixed-calendar `until_date` work until "now" passes that date, at which point the materializer's effective horizon resolves to a past timestamp and produces an empty occurrence set.

Relative-anchor helpers (e.g., `next_tuesday_anchor()` returning the next Tuesday after `Utc::now() + 1 day`) keep the test inputs in the same temporal position regardless of when the suite runs.

#### Scenario: Test anchor is computed from runtime, not hardcoded

- **WHEN** a contributor writes a test that calls `create_series_with_initial_materialization` and asserts an occurrence count or `materialized_through` value
- **THEN** the anchor SHALL be computed from `Utc::now()` (e.g., via a helper that finds the next occurrence-eligible weekday at a chosen time) and any dependent `until_date` SHALL be computed as a relative offset from that anchor

#### Scenario: Hardcoded calendar timestamps in materializer tests are a defect

- **WHEN** a contributor inspects a recurring-event test file or any `src/` file with `#[cfg(test)] mod tests`
- **THEN** instances of `Utc.with_ymd_and_hms(<year>, <month>, <day>, ...)` used as materializer inputs SHALL be treated as defects to be replaced with relative-anchor helpers; the rule is "no fixed-calendar inputs to the materializer in tests, regardless of where the test lives"

#### Scenario: Inline test modules in src/ follow the same rule

- **WHEN** an inline `#[cfg(test)] mod tests` block inside a service file (e.g., `src/service/event_admin_service.rs`) exercises the materializer or the service that wraps it
- **THEN** the test SHALL use runtime-relative anchors. The helpers MAY be duplicated per-file rather than shared until a third caller appears; premature extraction to a shared `src/service/test_helpers.rs` is not required

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

### Requirement: RecurringEventService rejects invalid recurrence input with a 4xx

`RecurringEventService::create_series_with_initial_materialization` SHALL return `AppError::BadRequest` (NOT `Internal`, NOT a panic) when either the supplied `Recurrence` fails its own `validate()` (e.g., a `Weekly` rule with an empty `weekdays` set, or an `interval` of 0), OR the combination of `template.start_time`, the rule, and `until_date` produces zero occurrences before the cutoff (e.g., `until_date` set before `template.start_time`). In both cases no `event_series` row SHALL be inserted; the call SHALL be side-effect free with respect to persisted state.

#### Scenario: Empty weekly weekday set returns BadRequest, no row written

- **WHEN** an admin POSTs the new-recurring-event form with a weekly rule whose weekday selector was left empty
- **THEN** `create_series_with_initial_materialization` SHALL return `Err(AppError::BadRequest(_))` and `SELECT COUNT(*) FROM event_series` SHALL be unchanged

#### Scenario: until_date earlier than start_time returns BadRequest

- **WHEN** the form submits `template.start_time = T` with `until_date = T - 6 days` (the operator mis-typed the end date)
- **THEN** the method SHALL return `Err(AppError::BadRequest(msg))` where the message contains "no occurrences"; no `event_series` or `events` rows SHALL be inserted

### Requirement: compute_occurrence_start_time validates index domain and series state

`RecurringEventService::compute_occurrence_start_time(series, index)` SHALL reject `index < 1` with `AppError::BadRequest("occurrence_index must be >= 1")`, SHALL reject the case where the series has zero corresponding `events` rows (anchor inference impossible) with `AppError::Internal("cannot infer series anchor — no occurrences exist")`, and SHALL reject an `index` beyond the generator's internal 10_000-entry cap with `AppError::BadRequest` whose message contains "beyond the series's generated occurrences". Each failure mode SHALL produce a typed error rather than a panic or generic 500.

#### Scenario: Zero or negative occurrence_index is a 4xx

- **WHEN** `compute_occurrence_start_time` is called with `index = 0` (or any value `< 1`)
- **THEN** the method SHALL return `Err(AppError::BadRequest(msg))` where `msg.contains("occurrence_index must be >= 1")`

#### Scenario: Series with no occurrences cannot extrapolate

- **WHEN** the caller invokes `compute_occurrence_start_time` against a series whose every `events` row has been deleted out-of-band
- **THEN** the method SHALL return `Err(AppError::Internal(msg))` where `msg.contains("cannot infer series anchor")`

#### Scenario: occurrence_index past the generator cap is a 4xx

- **WHEN** the caller asks for `compute_occurrence_start_time(series, 20_000)` on a weekly series (well past the 10_000-entry cap inside `generate_occurrences`)
- **THEN** the method SHALL return `Err(AppError::BadRequest(msg))` where `msg.contains("beyond the series's generated occurrences")`

