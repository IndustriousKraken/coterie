-- Guest (non-member) event registration: a guest price + a separate
-- "may non-members register" flag on the event, and an attendance row
-- that can belong to somebody who has no account.
--
-- Two columns, two questions. `guest_price_cents` answers "how much",
-- `guest_registration_enabled` answers "whether". Folding them into one
-- nullable price would make "the public attends free" indistinguishable
-- from "the public may not attend", and would reintroduce the
-- NULL-as-zero problem `member_price_cents` already rejects.
ALTER TABLE events ADD COLUMN guest_price_cents INTEGER NOT NULL DEFAULT 0;
ALTER TABLE events ADD COLUMN guest_registration_enabled BOOLEAN NOT NULL DEFAULT 0;

-- event_attendance is keyed `PRIMARY KEY (event_id, member_id)` with
-- member_id NOT NULL, so a guest has nowhere to sit. Rebuild it with a
-- surrogate id, a nullable member_id, and guest identity columns —
-- mirroring what migration 016 did to `payments` for public donations.
-- SQLite can't ALTER a PK or a CHECK, so this is the standard
-- table-rewrite recipe (same as 016 and 042).
--
-- `PRAGMA defer_foreign_keys = ON` rather than toggling `foreign_keys`:
-- the latter is inert inside a transaction, and sqlx::migrate wraps
-- every migration in one.
PRAGMA defer_foreign_keys = ON;

CREATE TABLE event_attendance_new (
    id TEXT PRIMARY KEY,
    event_id TEXT NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    -- Nullable: a guest row has no member account behind it.
    member_id TEXT REFERENCES members(id) ON DELETE CASCADE,
    guest_name TEXT,
    guest_email TEXT,
    status TEXT NOT NULL CHECK(status IN ('Registered', 'Waitlisted', 'Cancelled', 'PendingPayment')),
    registered_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    attended BOOLEAN NOT NULL DEFAULT 0,
    -- Carried forward from migration 022
    reminder_sent_at DATETIME,
    -- The event-fee payment holding this seat (migration 042). NULL for
    -- free RSVPs and free guest registrations.
    payment_id TEXT REFERENCES payments(id),
    -- Exactly one identity, enforced by the database rather than by
    -- service code that a future caller could forget.
    CHECK ( (member_id IS NOT NULL AND guest_email IS NULL)
         OR (member_id IS NULL     AND guest_email IS NOT NULL) ),
    -- One seat per identity per event. NULLs compare distinct in a
    -- SQLite UNIQUE index, so the member constraint doesn't collapse
    -- every guest row into one, and vice versa. These are the
    -- DB-level guarantee behind one-seat-per-identity: a concurrent
    -- double submission cannot produce two seats even if the service
    -- guard is bypassed.
    UNIQUE(event_id, member_id),
    UNIQUE(event_id, guest_email)
);

-- Existing rows are all member rows. The surrogate id is generated
-- here (SQLite has no uuid()) with the standard randomblob v4 recipe.
INSERT INTO event_attendance_new (
    id, event_id, member_id, status, registered_at, attended, reminder_sent_at, payment_id
)
SELECT
    lower(
        hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' ||
        substr(hex(randomblob(2)), 2) || '-' ||
        substr('89ab', abs(random()) % 4 + 1, 1) ||
        substr(hex(randomblob(2)), 2) || '-' || hex(randomblob(6))
    ),
    event_id, member_id, status, registered_at, attended, reminder_sent_at, payment_id
FROM event_attendance;

DROP TABLE event_attendance;
ALTER TABLE event_attendance_new RENAME TO event_attendance;

-- The refund path looks a seat up by payment id (migration 042).
CREATE INDEX idx_event_attendance_payment
    ON event_attendance(payment_id)
    WHERE payment_id IS NOT NULL;
