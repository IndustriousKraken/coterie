-- Add Hackathon to the event_type CHECK constraint
-- SQLite requires recreating the table to modify CHECK constraints

-- Step 1: Create new table with updated constraint
CREATE TABLE events_new (
    id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    event_type TEXT NOT NULL CHECK(event_type IN ('Meeting', 'Workshop', 'CTF', 'Social', 'Training', 'Hackathon')),
    visibility TEXT NOT NULL DEFAULT 'MembersOnly' CHECK(visibility IN ('Public', 'MembersOnly', 'AdminOnly')),
    start_time DATETIME NOT NULL,
    end_time DATETIME,
    location TEXT,
    max_attendees INTEGER,
    rsvp_required BOOLEAN NOT NULL DEFAULT 0,
    created_by TEXT NOT NULL REFERENCES members(id),
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Step 2: Copy data from old table
INSERT INTO events_new SELECT * FROM events;

-- Step 3: Drop old table
DROP TABLE events;

-- Step 4: Rename new table
ALTER TABLE events_new RENAME TO events;

-- Step 5: Recreate indexes
CREATE INDEX idx_events_start_time ON events(start_time);
CREATE INDEX idx_events_event_type ON events(event_type);
CREATE INDEX idx_events_visibility ON events(visibility);
