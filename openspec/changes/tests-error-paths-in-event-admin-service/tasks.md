## 1. Add error-path tests for `EventAdminService`

- [ ] 1.1 `update_one_errors_when_event_id_not_found` —
  call `svc.update_one(actor, Uuid::new_v4(), valid_input)` with a
  random UUID that doesn't exist. Assert
  `Err(AppError::NotFound(msg))` where `msg.contains("Event not
  found")`. Assert `audit_count(&pool, "update_event", _)` is `0` (the
  service short-circuits before the audit call).
- [ ] 1.2 `cancel_event_occurrence_errors_when_series_not_found` —
  call `svc.cancel_event_occurrence(actor, Uuid::new_v4(), 1, None)`
  with a random `series_id`. Assert
  `Err(AppError::NotFound(msg))` where `msg.contains("series")` and
  `msg.contains("not found")`.
- [ ] 1.3 `cancel_event_occurrence_errors_on_zero_index` — create
  a real series, then call `cancel_event_occurrence(actor, series_id,
  0, None)`. Assert `Err(AppError::BadRequest(msg))` where
  `msg.contains("occurrence_index must be >= 1")`.
- [ ] 1.4 `override_event_occurrence_errors_when_series_not_found` —
  same shape as 1.2, but for `override_event_occurrence`. Assert
  `Err(AppError::NotFound(_))`.
- [ ] 1.5 `override_event_occurrence_errors_on_zero_index` — same
  shape as 1.3, but for `override_event_occurrence`. Assert
  `Err(AppError::BadRequest(_))`.
- [ ] 1.6 `restore_event_occurrence_errors_when_series_not_found` —
  call `svc.restore_event_occurrence(actor, Uuid::new_v4(), 1)` with
  a random `series_id`. Assert `Err(AppError::NotFound(msg))` where
  `msg.contains("series")` (this hits the inline `find_by_id` path,
  not the `require_series_exists` helper).
- [ ] 1.7 `restore_event_occurrence_noop_when_no_exception_emits_audit` —
  create a real series with a real occurrence at index 2. Without ever
  cancelling or overriding it, call
  `svc.restore_event_occurrence(actor, series_id, 2)`. Assert
  `Ok(None)` is returned and `audit_count(&pool,
  "restore_event_occurrence", &format!("{}#2", series_id))` is `1`
  (the no-op still audits per the in-code comment "Audit the no-op so
  operator actions remain traceable").
