# Tasks

## 1. Resolve series visibility against an occurrence

- [ ] 1.1 In `src/web/portal/events.rs`, add a small private helper
  `series_visible_to_member(event_repo: &dyn EventRepository, series_id: Uuid,
  member: &Member) -> bool` that loads `list_series_occurrences(series_id)`
  and returns `occurrences.first().is_some_and(|e| e.visible_to_member(member))`
  — false for a series with no occurrences. Document that a series row carries
  no visibility of its own, so the rule is read against an occurrence, exactly
  as `load_enrollable` does in `src/web/templates/class_register.rs:153-173`.
- [ ] 1.2 Do NOT add a new visibility predicate to `EventSeries`; reuse
  `Event::visible_to_member` (`src/domain/event.rs:95`) so the `AdminOnly`
  rule keeps its single home.

## 2. Refuse the enroll on a class the member may not see

- [ ] 2.1 In `src/web/portal/events.rs::enroll_in_series`, after the series
  lookup succeeds (around line 514) and BEFORE `class_title` and
  `enrollment_service.enroll` are called, call the helper from 1.1 with
  `&current_user.member`.
- [ ] 2.2 When it returns false, return
  `axum::response::Html(render_class_error(&sid, 0, false, "Class not found"))`
  — byte-identical to the `Ok(None)` branch at line 507 (price `0`, `enrolled`
  false, same message), so an `AdminOnly` series id and a nonexistent one are
  indistinguishable.
- [ ] 2.3 Verify by reading the resulting control flow that no
  `series_enrollment` row, `event_attendance` row, payment row, or Checkout
  session can be created on this path, and that `class_title` (which would
  read the hidden title) is not reached.

## 3. Tests

- [ ] 3.1 Add an integration test
  `enroll_in_admin_only_series_is_refused_for_a_non_admin` (new file
  `tests/admin_only_class_enroll_test.rs`, following the router/session setup
  used by the existing admin-only events test): seed a paid recurring series
  whose occurrences are `EventVisibility::AdminOnly`, POST
  `/portal/api/series/:id/enroll` as a non-admin Active member, and assert the
  response body equals the body returned for a random nonexistent series id.
- [ ] 3.2 In the same test, assert no row was written: `series_enrollments`
  and `payments` for that member are empty, and `event_attendance` has no row
  for the member on any occurrence of the series.
- [ ] 3.3 Assert the hidden title does not appear anywhere in the response
  body.
- [ ] 3.4 Add `enroll_in_admin_only_series_succeeds_for_an_admin`: the same
  POST from a member with `is_admin` proceeds normally (enrollment held or
  registered, per the class's price).
- [ ] 3.5 Add `enroll_in_members_only_series_still_succeeds` so the change
  does not narrow ordinary member enrollment.
- [ ] 3.6 Run `cargo test --test admin_only_class_enroll_test`, then `cargo
  test`, and confirm nothing regressed.
