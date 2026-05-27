## 1. Add error-path tests for `RecurringEventService`

- [ ] 1.1 `create_series_errors_on_invalid_recurrence_rule` —
  build a `Recurrence::Weekly { interval: 1, weekdays: vec![] }` (empty
  weekday set is invalid per `Recurrence::validate`), call
  `create_series_with_initial_materialization`, assert
  `Err(AppError::BadRequest(_))` is returned and that the series row
  is NOT inserted (`series_repo.find_by_id(...)` returns `None`).
- [ ] 1.2 `create_series_errors_when_until_date_before_start_time` —
  build a valid weekly rule with `template.start_time = now + 7 days`
  and `until_date = Some(now + 1 day)`, call the same method, assert
  `Err(AppError::BadRequest(msg))` where `msg.contains("no
  occurrences")`. Assert no series row was inserted.
- [ ] 1.3 `compute_occurrence_start_time_errors_on_zero_index` —
  create a real series via `create_series_with_initial_materialization`,
  then call `compute_occurrence_start_time(&series, 0)`. Assert
  `Err(AppError::BadRequest(msg))` where `msg.contains("occurrence_index
  must be >= 1")`. Also exercise negative index (e.g., `-3`) and assert
  the same variant.
- [ ] 1.4 `compute_occurrence_start_time_errors_when_series_has_no_events` —
  create a series, manually `DELETE FROM events WHERE series_id = ?` to
  empty it, then call `compute_occurrence_start_time(&series, 1)`.
  Assert `Err(AppError::Internal(msg))` where `msg.contains("cannot
  infer series anchor")`.
- [ ] 1.5 `compute_occurrence_start_time_errors_on_index_beyond_horizon` —
  create a weekly series and call `compute_occurrence_start_time(&series,
  20_000)` (well past the 10_000-entry generator cap). Assert
  `Err(AppError::BadRequest(msg))` where `msg.contains("beyond the
  series's generated occurrences")`.
