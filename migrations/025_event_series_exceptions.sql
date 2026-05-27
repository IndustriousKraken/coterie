-- Per-occurrence exceptions for recurring event series.
--
-- A series's recurrence rule plus its template define every occurrence
-- "the same way." Real life isn't that tidy: a single occurrence
-- sometimes needs to be cancelled (the Dec 25 holiday) or overridden
-- (we're meeting in a different room this Tuesday). We don't want to
-- end the series + start a new one, and we don't want to manually edit
-- the `events` row only to have the next horizon-roll undo it.
--
-- Exception rows name "(series, occurrence_index)" as special:
--   - kind='cancelled' + override_payload IS NULL — the occurrence is
--     skipped on materialization; if a row already exists for it, the
--     service deletes that row.
--   - kind='overridden' + override_payload = JSON — the materializer
--     creates the row from the template as usual, then applies the
--     non-null fields from the payload on top.
--
-- "Restore" deletes the exception row and re-creates / resets the
-- underlying `events` row from the template.

CREATE TABLE IF NOT EXISTS event_series_exceptions (
    series_id TEXT NOT NULL REFERENCES event_series(id) ON DELETE CASCADE,
    occurrence_index INTEGER NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('cancelled', 'overridden')),
    override_payload TEXT,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    created_by TEXT NOT NULL REFERENCES members(id),
    audit_reason TEXT,
    PRIMARY KEY (series_id, occurrence_index)
);

CREATE INDEX IF NOT EXISTS idx_exceptions_series ON event_series_exceptions(series_id);
