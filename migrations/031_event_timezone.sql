-- Event timezone correctness: store each event's time as a local
-- wall-clock plus an IANA zone name, and derive the UTC instant at read
-- time. See openspec change `event-timezone-correctness`.
--
-- Before this, an admin's "7:00 PM" was stored as a naive wall-clock and
-- then mislabeled as UTC on the public/iCal read path, shifting every
-- remote viewer's rendered time by the org's offset. The stored values
-- are correct wall-clocks; only the interpretation was wrong. So this
-- migration is a pure ANNOTATION — it adds a zone column and backfills
-- it, and shifts no stored time value.

-- Org-wide default zone. IANA name; validated on write by SettingsService.
-- Default UTC reproduces today's behavior exactly.
INSERT INTO app_settings (key, value, value_type, category, description, is_sensitive) VALUES
    ('org.timezone', 'UTC', 'string', 'organization',
     'IANA timezone (e.g. America/New_York) events are scheduled in. Public and calendar output derive the UTC instant from this; changing it does not reinterpret existing events (each event freezes its own zone at creation).', 0);

-- Per-event zone, frozen at creation from org.timezone. Existing rows are
-- annotated with the current org zone — no time value is touched.
ALTER TABLE events ADD COLUMN timezone TEXT NOT NULL DEFAULT 'UTC';
UPDATE events SET timezone = (SELECT value FROM app_settings WHERE key = 'org.timezone');

-- The series carries the zone too (defaulted from org.timezone). Kept in
-- lockstep with its occurrences, which each store their own zone.
ALTER TABLE event_series ADD COLUMN timezone TEXT NOT NULL DEFAULT 'UTC';
UPDATE event_series SET timezone = (SELECT value FROM app_settings WHERE key = 'org.timezone');
