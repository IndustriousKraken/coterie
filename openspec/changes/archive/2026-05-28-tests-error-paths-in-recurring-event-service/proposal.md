## Why

`src/service/recurring_event_service.rs` has six explicit error returns that
no test in `tests/recurring_event_test.rs` exercises (verified by grepping
the test file for `is_err`, `unwrap_err`, `BadRequest`, `NotFound` —
zero hits). The happy paths are well covered (weekly materialization,
horizon extension, until-date capping, second-Wednesday rules, etc.) but
the failure branches that protect operators from data-shape mistakes are
entirely untested:

- `create_series_with_initial_materialization` (line 84): `rule.validate()`
  returns `Err(...)` → service maps to `AppError::BadRequest`. No test
  feeds an invalid `Recurrence` (e.g., a `Weekly` rule with an empty
  `weekdays` list) to assert this branch fires.
- `create_series_with_initial_materialization` (line 106): when the
  generator produces zero occurrences before the cutoff (e.g.,
  `until_date` is set BEFORE `template.start_time`), the service
  returns `AppError::BadRequest("Recurrence rule produced no
  occurrences before the cutoff")`. No test exercises this.
- `compute_occurrence_start_time` (line 305): `occurrence_index < 1`
  returns `AppError::BadRequest("occurrence_index must be >= 1")`. No
  test passes index 0 or a negative value.
- `compute_occurrence_start_time` (line 323): when the series has no
  events at all, anchor inference returns `AppError::Internal("cannot
  infer series anchor — no occurrences exist")`. No test deletes
  every occurrence and then calls this method.
- `compute_occurrence_start_time` (line 337): when `occurrence_index`
  is beyond the generator's 10_000-entry cap (which is the only way to
  reach the `times.get(idx)` `None` arm), returns
  `AppError::BadRequest("occurrence_index {} is beyond ...")`.

These branches all have user-visible consequences: an admin form that
submits a malformed recurrence rule, or a restore action against a
deleted-series, should get a coherent 4xx rather than an opaque 500
or — worse — silently succeed against a stale path.

## What Changes

Add `#[tokio::test]` cases in `src/service/recurring_event_service.rs`'s
inline `#[cfg(test)] mod tests` block (or a new module if one doesn't
exist yet) that drive each of the five branches above and assert on
the specific `AppError` variant and message substring.

Per the `admin-events` capability's "tests use runtime-relative anchors"
rule, all inputs SHALL be computed from `Utc::now()` at test time, not
from fixed calendar dates.

## Impact

- `src/service/recurring_event_service.rs` — add an inline
  `#[cfg(test)] mod tests` block (the file currently has none) with
  five new `#[tokio::test]` functions and the shared in-memory
  SQLite/repo plumbing required to drive them. Alternative landing
  site: a sibling `tests/recurring_event_error_paths_test.rs` —
  either is acceptable as long as the tests run.
- No production code change. No existing tests are modified or
  deleted.
