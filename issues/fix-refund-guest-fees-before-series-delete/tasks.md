## 1. Gate the bulk scopes on outstanding event fees, not on the member price

- [ ] 1.1 In `src/web/portal/admin/events/single.rs::admin_delete_event`,
  add `State(payment_repo): State<Arc<dyn PaymentRepository>>` to the
  handler's extractors (`AppState` already provides it — see
  `src/web/portal/admin/events/roster.rs::admin_roster_release_seat` for
  the same extractor) and import `crate::repository::PaymentRepository`.
- [ ] 1.2 Replace the `if event.is_paid_for_members()` test at
  `src/web/portal/admin/events/single.rs:1101` with a check across the
  whole series: for each occurrence returned by
  `event_repo.list_series_occurrences(sid)`, call
  `payment_repo.list_completed_event_fees(occurrence.id)`; if any
  occurrence returns a non-empty list, return the existing
  `partials::admin_alert("error", …, false)` refusal. Reword the message
  so it no longer says "member price" — e.g. "Attendees have paid for
  individual sessions of this series. Delete those sessions one at a time
  so each one's attendees are refunded first." Move the `let sid =
  series_id.unwrap();` binding above this check so the occurrence lookup
  can use it.
- [ ] 1.3 Fail closed on a lookup error: if
  `list_series_occurrences` or `list_completed_event_fees` returns `Err`,
  return an `admin_alert("error", …)` refusal rather than falling through
  to the delete. An unreadable answer must not authorise destroying a
  roster.
- [ ] 1.4 Update the block comment above the guard
  (`src/web/portal/admin/events/single.rs:1094-1100`) to describe the new
  rule: the bulk scopes drop occurrences this handler never sees, so they
  are refused whenever ANY occurrence of the series still carries a
  `Completed` event fee — member or guest — because those payments can
  only be refunded from the per-occurrence path.

## 2. Regression tests

- [ ] 2.1 Add a test `series_delete_refused_when_a_guest_paid_an_occurrence`
  in `src/web/portal/admin/events/single.rs`'s test module: materialize a
  series whose occurrences have `member_price_cents = 0` and
  `guest_price_cents = 2500`, insert a `Completed` `EventFee` payment with
  a `Payer::PublicDonor` payer against one occurrence plus its
  `event_attendance` row, POST the delete with `scope=delete_series`, and
  assert the response is the refusal alert AND that the series row and
  every occurrence still exist.
- [ ] 2.2 Add `series_delete_allowed_when_no_occurrence_was_paid`: same
  series shape with guest pricing set but no payments, POST with
  `scope=delete_series`, and assert the series and its occurrences are
  gone. Proves the guard keys on payments, not on the price column.
- [ ] 2.3 Add `end_series_refused_when_a_member_paid_an_occurrence` to keep
  the previously-covered member case pinned under the new rule: a
  `Completed` `EventFee` payment with a `Payer::Member` payer against one
  occurrence, POST with `scope=end_series`, assert the refusal and that no
  occurrence was deleted.
