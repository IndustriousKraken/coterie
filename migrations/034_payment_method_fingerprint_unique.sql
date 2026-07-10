-- Backstop the saved-card backfill's fingerprint de-dup at the DB level.
--
-- Migration 033 added `card_fingerprint` but the backfill's de-dup is a
-- non-atomic app-side read-then-write (find_by_member -> compare ->
-- create). Two concurrent backfill runs, a double-clicked import, or a
-- backfill overlapping a live SetupIntent card-add can each pass the
-- check and insert a duplicate card row. Payments are backstopped by the
-- partial unique index on `stripe_payment_id`; cards had no equivalent.
--
-- Scope the uniqueness per member (member_id, card_fingerprint) so the
-- same physical card can legitimately exist under two different members;
-- WHERE card_fingerprint IS NOT NULL so pre-existing / SetupIntent rows
-- without a recorded fingerprint are unaffected.
CREATE UNIQUE INDEX IF NOT EXISTS idx_payment_methods_member_fingerprint
    ON payment_methods (member_id, card_fingerprint)
    WHERE card_fingerprint IS NOT NULL;
