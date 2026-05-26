## 1. Database migration

- [x] 1.1 Create `migrations/NNNN_event_series_exceptions.sql` with the schema from `design.md` D1.
- [x] 1.2 Run migration against an in-memory DB in a smoke test to confirm syntax + indexes.

## 2. Domain types

- [x] 2.1 In `src/domain/event.rs` (or a new `src/domain/event_exception.rs`): `OccurrenceException { series_id, occurrence_index, kind, override_payload, created_at, created_by, audit_reason }`.
- [x] 2.2 `pub enum OccurrenceExceptionKind { Cancelled, Overridden }` with serde + sqlx-compatible string repr.
- [x] 2.3 `OccurrenceOverride { title: Option<String>, description: Option<String>, start_time: Option<DateTime<Utc>>, end_time: Option<DateTime<Utc>>, location: Option<String>, max_attendees: Option<i32>, rsvp_required: Option<bool>, image_url: Option<String> }` with `serde(default)` on each field.
- [x] 2.4 `OccurrenceOverride::apply(self, target: &mut Event)` — non-`None` fields overwrite the target's fields.

## 3. Repository

- [x] 3.1 Extend `EventSeriesRepository` trait with:
  - `insert_exception(exception: OccurrenceException) -> Result<()>`
  - `delete_exception(series_id: Uuid, occurrence_index: i32) -> Result<()>`
  - `find_exception(series_id: Uuid, occurrence_index: i32) -> Result<Option<OccurrenceException>>`
  - `list_exceptions_for_series(series_id: Uuid) -> Result<Vec<OccurrenceException>>`
- [x] 3.2 Implement in the concrete `EventSeriesRepository` impl.
- [x] 3.3 Repo tests covering insert/delete/find/list against in-memory SQLite.

## 4. Service methods

- [x] 4.1 Add `cancel_event_occurrence(actor_id, series_id, occurrence_index, reason)` to `EventAdminService`. Implementation:
  - Insert exception row (`kind = Cancelled`, payload = `None`).
  - Delete the corresponding `events` row (find by `(series_id, occurrence_index)`).
  - Emit audit row `action = "cancel_event_occurrence"`, `entity_type = "event"`, `entity_id = events_row_id`, `old_value = Some(occurrence_title_or_index_as_string)`, `new_value = None`.
  - Idempotency: if exception already exists, log + skip the events-row delete (it's already gone).
- [x] 4.2 Add `override_event_occurrence(actor_id, series_id, occurrence_index, overrides, reason)`. Implementation:
  - Insert exception row (`kind = Overridden`, `override_payload = serde_json::to_string(&overrides)?`).
  - Update the `events` row: fetch it, apply `overrides`, write back.
  - Emit audit row `action = "override_event_occurrence"`.
  - Return the updated `Event`.
- [x] 4.3 Add `restore_event_occurrence(actor_id, series_id, occurrence_index)`. Implementation:
  - Find the exception. If absent → log + no-op.
  - If `Cancelled` → re-materialize the single occurrence from the series template via existing materializer logic for one index. Insert the row.
  - If `Overridden` → reset the `events` row to the series template (re-read template, recompute start_time, overwrite the row).
  - Delete the exception row.
  - Emit audit row `action = "restore_event_occurrence"`.
  - Return `Some(event)` for cancelled-restore (a new row), `None` for overridden-restore (just an update).

## 5. Materializer change

- [x] 5.1 In `RecurringEventService::materialize_horizon` (or whichever method handles the daily roll-forward + initial materialization): before INSERTing each new `(series_id, occurrence_index)` row, query `event_series_exceptions` for that pair.
- [x] 5.2 If `kind = Cancelled` exception exists → skip the insert.
- [x] 5.3 If `kind = Overridden` exception exists → build the event from the series template AS USUAL, then apply `override_payload` overrides via `OccurrenceOverride::apply`, then insert.
- [x] 5.4 If no exception → existing behavior (insert from template).

## 6. Admin handlers

- [x] 6.1 New routes under `src/web/portal/admin/events.rs` (or wherever event admin routes live):
  - `POST /portal/admin/events/series/:id/occurrences/:index/cancel` — extracts the form's optional reason; calls `cancel_event_occurrence`.
  - `POST /portal/admin/events/series/:id/occurrences/:index/override` — multipart form parsed into `OccurrenceOverride`; calls `override_event_occurrence`.
  - `POST /portal/admin/events/series/:id/occurrences/:index/restore` — calls `restore_event_occurrence`.
- [x] 6.2 Each handler returns an HTMX fragment that the event-series detail page swaps in to update the occurrence row in place.
- [x] 6.3 Wire CSRF + admin middleware (inherits the portal router's tiers).

## 7. UI

- [x] 7.1 Update the event-series detail page template (`templates/admin/event_series_detail.html` or equivalent). For each future occurrence row, add HTMX buttons:
  - "Cancel" — `hx-post` to `.../cancel`, with a `hx-prompt` for the reason.
  - "Edit just this" — `hx-get` to load the override modal.
- [x] 7.2 For each occurrence with an existing exception, render a "Cancelled — reason — restore" or "Overridden — restore" indicator.
- [x] 7.3 New template `templates/admin/event_occurrence_override_form.html` for the override modal. Includes only the overridable fields (8 of them).
- [x] 7.4 Past occurrences SHALL NOT render the Cancel / Edit-just-this buttons (template conditional on `occurrence.start_time > now`).

## 8. Integration tests

- [x] 8.1 `cancel_event_occurrence_writes_exception_and_deletes_row` — happy path.
- [x] 8.2 `cancelled_occurrence_does_not_reappear_after_materializer_run` — schedule a series, cancel one, advance time / call materializer, assert the occurrence is still absent.
- [x] 8.3 `override_event_occurrence_updates_row_and_writes_exception` — happy path.
- [x] 8.4 `overridden_occurrence_survives_series_edit` — override one occurrence, then series-edit-future with re-materialization, assert override still applies.
- [x] 8.5 `restore_cancelled_recreates_row` — cancel + restore round-trip.
- [x] 8.6 `restore_overridden_resets_to_template` — override + restore round-trip; assert overridden fields reset to series defaults.
- [x] 8.7 `cancel_then_cancel_is_idempotent` — call cancel twice, no error.
- [x] 8.8 `audit_rows_emitted_for_each_action` — confirm audit-log entries for cancel/override/restore.

## 9. Validation

- [x] 9.1 `cargo build --features test-utils` — clean.
- [x] 9.2 `cargo test --features test-utils` — all tests pass (existing unrelated `member_template_snapshots` failures pre-date this change — HTMX/Alpine CDN-vs-local drift in `templates/layouts/base.html`).
- [x] 9.3 `cargo clippy --features test-utils -- --deny warnings` — no new errors introduced by this change (baseline of 66 pre-existing clippy errors unchanged).
- [x] 9.4 `cargo fmt --check` — new files formatted via `rustfmt --edition 2021`; broader workspace fmt drift is pre-existing.
- [ ] 9.5 Manual UI check: load the admin series detail page, click Cancel on one, verify the row updates; click Restore, verify it comes back; click Edit-just-this, change the location, verify the change is reflected. **(Sandbox cannot perform manual browser interactions; verified via integration tests at the service layer.)**
