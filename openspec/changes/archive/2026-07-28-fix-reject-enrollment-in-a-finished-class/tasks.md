## 1. Refuse enrollment when no session remains

- [x] 1.1 In `src/service/series_enrollment_service.rs`, add a private
  method to `impl SeriesEnrollmentService`:
  `async fn require_remaining_session(&self, series_id: Uuid) -> Result<()>`.
  It reads `self.event_repo.list_series_occurrences(series_id).await?` and
  returns `Ok(())` when any occurrence has `occurrence.start_utc() >
  chrono::Utc::now()`, otherwise `Err(AppError::BadRequest(...))` with a
  message naming the reason (e.g. "This class has finished — every session
  has already started, so there is nothing left to enroll in."). Use
  `start_utc()`, never the naive `start_time`, for the same timezone
  reason `seat_future_occurrences` documents.
- [x] 1.2 Call it as the first statement of
  `SeriesEnrollmentService::enroll` (before the `MemberStatus` check) and
  of `SeriesEnrollmentService::enroll_guest` (before `guest_attendee`).
  Both paths — free and paid — are covered, so a free finished class also
  stops handing out enrollments that seat nobody.
- [x] 1.3 Do NOT add the guard to
  `src/web/portal/admin/events/enrollments.rs::record_enrollment_payment`
  or to `seat_future_occurrences` / `confirm_enrollment_for_payment`. The
  admin at-the-door and comp path deliberately bypasses these guards, and
  the webhook confirm path must still be able to settle a checkout that
  was legitimately started while a session remained.
- [x] 1.4 Extend the module doc at the top of
  `src/service/series_enrollment_service.rs` with one line recording the
  new invariant: a pass is only sellable while at least one session has
  not started.

## 2. Regression tests

- [x] 2.1 Add `enroll_guest_is_refused_when_every_session_has_started` in
  `src/service/series_enrollment_service.rs`'s test module (or the
  existing paid-events test harness): a bounded paid series whose
  occurrences all start in the past, `enroll_guest` returns
  `Err(AppError::BadRequest(_))`, and assert no `series_enrollment` row,
  no `payments` row, and no Checkout session were created.
- [x] 2.2 Add `enroll_member_is_refused_when_every_session_has_started`
  for the `enroll` path with the same assertions.
- [x] 2.3 Add `enroll_succeeds_with_one_session_remaining`: a series with
  five past occurrences and one future one; `enroll` returns
  `RegistrationOutcome::Checkout` (paid) at the full pass price and a
  `PendingPayment` enrollment exists. Pins that the floor did not become
  proration.
- [x] 2.4 Add `free_finished_class_enrollment_is_refused`: same shape as
  2.1 but with `member_price_cents = 0` and `guest_price_cents = 0`,
  asserting the free short-circuit is also refused and creates no
  `series_enrollment` row.
