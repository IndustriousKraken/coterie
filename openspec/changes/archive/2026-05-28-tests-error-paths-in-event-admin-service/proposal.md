## Why

`src/service/event_admin_service.rs` has an extensive happy-path test
suite (~15 `#[tokio::test]` cases in the inline `tests` module) covering
create, update, delete, cancel, override, restore, and materializer
re-application of overrides. None of those tests assert on the typed
error returns. Grepping the file for `unwrap_err` / `is_err` /
`assert!(...err)` yields one hit, which is asserting `result.is_none()`
on an `Option`, not a failure case.

The unguarded error paths are:

- `update_one` (line 190): `event_repo.find_by_id` returns `None` →
  `AppError::NotFound("Event not found")`. Caller passes a UUID that
  doesn't exist (stale form, deleted-elsewhere) — should 4xx, not 500.
- `cancel_event_occurrence` (line 376) + `override_event_occurrence`
  (line 437) + `restore_event_occurrence` (line 503): `occurrence_index
  < 1` returns `AppError::BadRequest("occurrence_index must be >= 1")`.
- `cancel_event_occurrence` + `override_event_occurrence` via
  `require_series_exists` (line 575): unknown `series_id` returns
  `AppError::NotFound(format!("series {} not found", series_id))`.
- `restore_event_occurrence` (line 501): unknown `series_id` returns
  `AppError::NotFound(format!("series {} not found", series_id))` from
  the inline `find_by_id`-then-`ok_or_else` chain.
- `restore_event_occurrence` (line 514–526): when no exception exists for
  the `(series, index)` pair, the method emits an audit row and
  returns `Ok(None)` — the "no-op + audit" branch. Currently untested,
  even though the `audit_rows_emitted_for_each_action` test only
  exercises the cancel/override/restore happy paths.

Each of these is reachable from a real admin handler (`POST
/portal/admin/events/series/:id/occurrences/:idx/cancel` etc.); a stale
admin tab or a hand-crafted URL can plausibly produce any of them.

## What Changes

Extend the existing `#[cfg(test)] mod tests` block in
`src/service/event_admin_service.rs` with seven `#[tokio::test]`
functions that exercise these error/no-op branches and assert on the
specific `AppError` variant + message substring (or audit-row presence
for the no-op case).

## Impact

- `src/service/event_admin_service.rs` — add seven new `#[tokio::test]`
  functions to the inline test module. Reuses the existing `make_service`
  / `make_actor` / `fresh_pool` / `audit_count` helpers; no new
  infrastructure required.
- No production code change. No existing tests are modified or
  deleted.
