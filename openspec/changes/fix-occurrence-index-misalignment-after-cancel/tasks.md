## 1. Number new occurrences by stream position, not surviving-row MAX

- [ ] 1.1 In `src/service/recurring_event_service.rs::extend_horizon`, remove the `next_index` computed from `self.event_repo.max_occurrence_index_for_series(series.id)` (lines ~235-239).
- [ ] 1.2 Derive a stream-aligned base index instead. Using the already-computed `anchor` and parsed `rule`, count how many stream occurrences fall at or before `series.materialized_through` — e.g. `let base_index = generate_occurrences(anchor, &rule, anchor, series.materialized_through + Duration::seconds(1)).len() as i32;`. This counts the stream positions consumed through `materialized_through` regardless of whether their `events` rows were later cancelled/deleted.
- [ ] 1.3 In the insertion loop (lines ~247-248), set `let occurrence_index = base_index + (i as i32) + 1;` so new occurrences continue the stream numbering. Confirm this matches how `create_series_with_initial_materialization` assigns `(idx + 1)` (lines ~136-148), i.e. an index is consumed per stream position even when that position is a `Cancelled` exception.
- [ ] 1.4 Verify the exception lookup `find_exception(series.id, occurrence_index)` (lines ~249-252) now receives the correct stream index, so a `Cancelled` exception at a *past* boundary index is not re-collided with a *future* occurrence.

## 2. Spec update

- [ ] 2.1 Apply the `MODIFIED Requirements` block in `specs/event-admin-service/spec.md` from this change to `openspec/specs/event-admin-service/spec.md`, replacing the existing "Materializer respects per-occurrence exceptions" requirement so it additionally mandates stream-position-aligned `occurrence_index` assignment and includes the boundary-cancel scenario.

## 3. Tests

- [ ] 3.1 Add a test `extend_horizon_after_cancelling_boundary_occurrence_keeps_indices_aligned` (in `tests/recurring_event_test.rs` or the existing recurring-event test module): create a weekly series materialized through occurrence N (the horizon boundary), cancel occurrence N via `cancel_event_occurrence`, then call `extend_horizon` with a target past several more occurrences. Assert (a) the occurrence at stream position N+1 IS created (not skipped), and (b) the newly created rows carry `occurrence_index = N+1, N+2, …` (stream-aligned), not `N, N+1, …`.
- [ ] 3.2 Add a regression assertion that with no cancellations, `extend_horizon` still assigns contiguous indices continuing from the prior batch (guards against off-by-one in the new base-index computation).
