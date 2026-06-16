## Why

`RecurringEventService::extend_horizon` assigns `occurrence_index` to newly materialized occurrences by deriving the next index from `MAX(occurrence_index)` over the *surviving* `events` rows:

```rust
// src/service/recurring_event_service.rs:235-248
let next_index = self
    .event_repo
    .max_occurrence_index_for_series(series.id)   // SELECT MAX(occurrence_index) FROM events WHERE series_id = ?
    .await?
    .unwrap_or(0);
...
for (i, start) in new_times.iter().enumerate() {
    let occurrence_index = next_index + (i as i32) + 1;
```

But `occurrence_index` is meant to be the **1-based position in the recurrence stream**, and that is how initial materialization assigns it: `create_series_with_initial_materialization` numbers by stream position and consumes an index even for cancelled occurrences (`src/service/recurring_event_service.rs:136-148` — the loop uses `(idx + 1)` and only `continue`s for a cancelled index *after* consuming it). The materializer then keys exception lookups on that index (`Materializer respects per-occurrence exceptions`, `openspec/specs/event-admin-service/spec.md:86-106`).

These two index sources diverge after a boundary cancellation. `cancel_event_occurrence` inserts a `Cancelled` exception for the index AND **hard-deletes the `events` row** (`src/service/event_admin_service.rs:393-407`) without adjusting `series.materialized_through`. So if the highest-numbered materialized occurrence is cancelled, `MAX(occurrence_index)` drops below the true stream position of `materialized_through`.

Triggering scenario (weekly series, admin-reachable):
1. A series materializes occurrences 1..52; `materialized_through` = occurrence #52's start.
2. An admin cancels occurrence #52 (the boundary occurrence). Its `events` row is deleted, a `Cancelled` exception is stored for index 52, and `materialized_through` is left unchanged. Now `MAX(occurrence_index) = 51`.
3. The daily horizon-roll calls `extend_horizon`. It generates new occurrences starting at `materialized_through + 1s` — i.e. the stream's #53, #54, … — but `next_index = 51`, so the first new occurrence is assigned `occurrence_index = 52`.
4. The loop calls `find_exception(series, 52)`, finds the `Cancelled` exception left by step 2, and **skips the occurrence** (`continue`, `src/service/recurring_event_service.rs:254-259`).

Harm: a legitimate future occurrence is silently never created (it never appears on the calendar / RSVP list), and every subsequent index is shifted down by the number of trailing cancelled occurrences. The shift also corrupts `restore_event_occurrence`'s index-based start-time recomputation for later occurrences. This is data corruption that compounds on each horizon-roll, and it directly violates the spec's guarantee that the materializer consults `event_series_exceptions` for the correct `(series_id, occurrence_index)` pair.

## What Changes

Make `extend_horizon` number new occurrences by their **true stream position** rather than by `MAX(occurrence_index)` of surviving rows, so a cancelled (and deleted) boundary occurrence no longer shifts later indices.

Concretely: compute the index offset from the recurrence stream itself. The materializer already re-derives the `anchor` (the first occurrence's start) and generates from it. Generate (or count) occurrences from the `anchor` up to `materialized_through` to determine how many stream positions precede the first new occurrence, and number the new occurrences from there — e.g. `base_index = count_of_stream_occurrences_through(materialized_through)`, then `occurrence_index = base_index + i + 1`. Equivalently, persist the last-emitted stream index on the series row and advance it. Either approach keeps the index aligned with the stream regardless of how many trailing occurrences were cancelled/deleted, matching `create_series_with_initial_materialization`.

Update the `event-admin-service` spec's "Materializer respects per-occurrence exceptions" requirement to state that occurrence indices SHALL track the recurrence stream position (not surviving-row `MAX`), and add a scenario for the boundary-cancel case.

## Impact

- `src/service/recurring_event_service.rs` — replace the `max_occurrence_index_for_series`-based `next_index` in `extend_horizon` with a stream-position-derived base index. The `anchor`/`generate_occurrences` machinery needed to compute it is already present in the function.
- `src/repository/event_repository.rs` — `max_occurrence_index_for_series` may become unused by this path; leave it if other callers exist, otherwise it can be removed in a follow-up (not required by this change).
- `openspec/specs/event-admin-service/spec.md` — MODIFY "Materializer respects per-occurrence exceptions" to require stream-aligned indexing and add the boundary-cancel scenario.
- Tests: a new test that cancels the last materialized occurrence of a series, then extends the horizon, and asserts the next real occurrence is created (not skipped) and that indices stay aligned with the stream.
