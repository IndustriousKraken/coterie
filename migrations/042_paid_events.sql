-- Paid events: a member price on the event, a payment link + an
-- in-flight state on the attendance row, and an event reference on the
-- payment row.
--
-- `member_price_cents` is NOT NULL DEFAULT 0 and every existing event
-- backfills to 0. Zero is stored as zero rather than NULL because NULL
-- means "unknown", and using it as a sentinel for free breaks the
-- queries nobody thinks to test: `WHERE member_price_cents = 0` matches
-- no free event, `<= 2000` omits them (NULL comparison is unknown, not
-- true), and SUM/AVG/ORDER BY each skip them — all silently.
ALTER TABLE events ADD COLUMN member_price_cents INTEGER NOT NULL DEFAULT 0;

-- `payments.event_id` makes the event reachable FROM the payment. A
-- `charge.refunded` webhook arrives with a payment id and nothing else,
-- so releasing the right seat needs the event id on the payment itself.
-- Deliberately NOT a foreign key: `payments` is the money ledger and
-- outlives the event (deleting a paid event refunds and removes it, but
-- the refunded rows stay). A FK would either block the delete or NULL
-- the column out from under a settled financial record.
ALTER TABLE payments ADD COLUMN event_id TEXT;

CREATE INDEX idx_payments_event
    ON payments(event_id)
    WHERE event_id IS NOT NULL;

-- event_attendance needs a widened status CHECK ('PendingPayment') and a
-- nullable payment_id. SQLite can't ALTER a CHECK constraint, so this is
-- the standard table-rewrite recipe (same as migration 016).
--
-- `PRAGMA defer_foreign_keys = ON` rather than toggling `foreign_keys`:
-- the latter is inert inside a transaction, and sqlx::migrate wraps every
-- migration in one. Deferred checks tolerate the FK breakage during the
-- rewrite and verify at COMMIT, by which time the rename restored it.
PRAGMA defer_foreign_keys = ON;

CREATE TABLE event_attendance_new (
    event_id TEXT NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    member_id TEXT NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    -- 'PendingPayment' is new: a seat held while the member is at Stripe.
    status TEXT NOT NULL CHECK(status IN ('Registered', 'Waitlisted', 'Cancelled', 'PendingPayment')),
    registered_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    attended BOOLEAN NOT NULL DEFAULT 0,
    -- Carried forward from migration 022
    reminder_sent_at DATETIME,
    -- New: the event-fee payment holding this seat. NULL for free RSVPs.
    payment_id TEXT REFERENCES payments(id),
    PRIMARY KEY (event_id, member_id)
);

INSERT INTO event_attendance_new (
    event_id, member_id, status, registered_at, attended, reminder_sent_at
)
SELECT
    event_id, member_id, status, registered_at, attended, reminder_sent_at
FROM event_attendance;

DROP TABLE event_attendance;
ALTER TABLE event_attendance_new RENAME TO event_attendance;

-- The refund path looks a seat up by payment id.
CREATE INDEX idx_event_attendance_payment
    ON event_attendance(payment_id)
    WHERE payment_id IS NOT NULL;
