-- Recurring events.
--
-- Coterie uses an "instance-explosion" model: every occurrence is a real
-- row in `events`, with `series_id` pointing back to the row in
-- `event_series` that holds the recurrence rule. This means every
-- existing query (RSVPs, list pages, iCal export, search, Discord
-- push) keeps working without modification — an occurrence is just an
-- event that knows it has siblings.
--
-- The `event_series` row carries:
--   - the recurrence rule (a small JSON struct, parsed by the Rust
--     `Recurrence` enum — not full RFC 5545; see src/domain/recurrence.rs)
--   - `until_date` — optional end of the series; the materializer
--     stops at this point
--   - `materialized_through` — how far ahead occurrences have been
--     pre-generated. The daily horizon-extension job rolls this
--     forward; on creation we materialize 12 months ahead.
--
-- Edit and cancel semantics work directly on `events` rows:
--   - Edit one occurrence    = UPDATE that row
--   - Edit this and future   = UPDATE all rows in the series with
--                              start_time >= the chosen occurrence
--   - Cancel one occurrence  = DELETE that row (hard delete; past
--                              occurrences stay)
--   - End the series here    = DELETE all rows with start_time > now,
--                              SET event_series.until_date = today
--   - Delete the series      = DELETE all rows, then the series row

CREATE TABLE IF NOT EXISTS event_series (
    id TEXT PRIMARY KEY,
    -- Discriminant for the rule_json blob: one of
    --   'weekly_by_day', 'monthly_by_dom', 'monthly_by_weekday'
    -- New kinds (e.g. yearly) get added here without touching the
    -- column shape.
    rule_kind TEXT NOT NULL,
    -- The serialized rule parameters. Format depends on rule_kind;
    -- the Rust side owns parsing via a tagged-enum serde derive.
    rule_json TEXT NOT NULL,
    -- Optional last-occurrence cutoff. NULL means "keep rolling
    -- forward forever" (the operator wants an open-ended series like
    -- a weekly meetup that never ends).
    until_date DATETIME,
    -- The latest occurrence start_time we've materialized. The daily
    -- horizon-extension job uses this as the lower bound when
    -- generating new rows. Initially set at series-create time when
    -- we materialize ~12 months ahead.
    materialized_through DATETIME NOT NULL,
    created_by TEXT NOT NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (created_by) REFERENCES members(id) ON DELETE CASCADE
);

ALTER TABLE events ADD COLUMN series_id TEXT
    REFERENCES event_series(id) ON DELETE CASCADE;

-- 1-based position within the series. Useful for "occurrence #5 of
-- the Tuesday Coffee series" displays and for stable iCal UIDs
-- (UID = `events.id`, but the index keeps things readable in admin
-- views and audit logs).
ALTER TABLE events ADD COLUMN occurrence_index INTEGER;

CREATE INDEX IF NOT EXISTS idx_events_series ON events(series_id, start_time);
CREATE INDEX IF NOT EXISTS idx_event_series_until ON event_series(until_date);
