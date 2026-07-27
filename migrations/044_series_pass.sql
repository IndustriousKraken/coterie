-- Series pass: one payment enrolls somebody in every remaining
-- occurrence of a bounded recurring series ("Intro to Lockpicking, six
-- Tuesdays, $120").
--
-- The pricing columns mirror what migrations 042/043 put on `events`,
-- with the same semantics: NOT NULL DEFAULT 0, where 0 means free and
-- NULL is never used as a sentinel for free. Every existing series
-- backfills to 0, so an org that never prices a series sees no change.
ALTER TABLE event_series ADD COLUMN member_price_cents INTEGER NOT NULL DEFAULT 0;
ALTER TABLE event_series ADD COLUMN guest_price_cents INTEGER NOT NULL DEFAULT 0;
ALTER TABLE event_series ADD COLUMN guest_registration_enabled BOOLEAN NOT NULL DEFAULT 0;

-- Capacity for the CLASS, not for a night of it: twelve seats in the
-- six-week course, not twelve seats each Tuesday. Nullable on purpose —
-- unlike the prices, an absent capacity genuinely means "no limit"
-- rather than "zero", so there is no sentinel problem to avoid here.
ALTER TABLE event_series ADD COLUMN max_enrollments INTEGER;

-- `payments.series_id` makes the series reachable FROM the payment, for
-- exactly the reason `payments.event_id` exists (migration 042): a
-- `charge.refunded` webhook arrives with a payment id and nothing else,
-- so cancelling the right enrollment needs the series id on the payment
-- row itself. Deliberately NOT a foreign key — `payments` is the money
-- ledger and outlives the series.
ALTER TABLE payments ADD COLUMN series_id TEXT;

CREATE INDEX idx_payments_series
    ON payments(series_id)
    WHERE series_id IS NOT NULL;

-- Who bought a pass. Shape follows `event_attendance` post-043: a
-- surrogate id, a nullable member_id, guest identity columns, and the
-- same exactly-one-identity CHECK — a pass belongs to a member or to a
-- guest, never both and never neither.
CREATE TABLE series_enrollment (
    id TEXT PRIMARY KEY,
    series_id TEXT NOT NULL REFERENCES event_series(id) ON DELETE CASCADE,
    member_id TEXT REFERENCES members(id) ON DELETE CASCADE,
    guest_name TEXT,
    guest_email TEXT,
    -- Same lifecycle as a single paid seat: 'PendingPayment' while the
    -- buyer is at Stripe, 'Registered' once the completion webhook
    -- confirms, 'Cancelled' on refund.
    status TEXT NOT NULL CHECK(status IN ('Registered', 'Waitlisted', 'Cancelled', 'PendingPayment')),
    enrolled_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    -- The series-pass payment holding this enrollment. NULL for a free
    -- class and for the instant between claiming and creating the session.
    payment_id TEXT REFERENCES payments(id),
    CHECK ( (member_id IS NOT NULL AND guest_email IS NULL)
         OR (member_id IS NULL     AND guest_email IS NOT NULL) ),
    -- One enrollment per identity per series, as a database guarantee
    -- rather than a service check a concurrent double-submit can race.
    -- NULLs compare distinct in a SQLite UNIQUE index, so the member
    -- constraint doesn't collapse every guest row into one.
    UNIQUE(series_id, member_id),
    UNIQUE(series_id, guest_email)
);

-- The refund + confirmation paths both look an enrollment up by payment id.
CREATE INDEX idx_series_enrollment_payment
    ON series_enrollment(payment_id)
    WHERE payment_id IS NOT NULL;
