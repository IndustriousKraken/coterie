-- Make a refunded membership payment able to give back the dues it
-- granted. `dues_extended_at` (migration 014) records THAT a payment
-- extended dues, never BY HOW MUCH — so a refund had nothing to reverse
-- and the member kept a membership whose fee went back to them.
--
-- `dues_extension_seconds` is the exact delta the extension applied
-- (new dues_paid_until − the date it extended from). Storing the delta
-- rather than the before/after pair is what makes retraction a
-- subtraction instead of a reset: a member holding dues from several
-- payments loses only the refunded one's contribution.
--
-- `dues_retracted_at` is the retraction's own idempotency anchor, the
-- mirror of `dues_extended_at`. The admin refund path and its Stripe
-- webhook echo can both reach retraction; whoever stamps this column
-- first owns it and later callers no-op. It is a separate column rather
-- than clearing the delta so the financial record keeps what was
-- granted as well as when it was taken back.
--
-- Rows written before this migration have a NULL delta and therefore
-- retract nothing — deliberately. Repairing already-refunded-but-
-- unretracted windows is an operator decision, not a migration guessing
-- at intent.

ALTER TABLE payments ADD COLUMN dues_extension_seconds INTEGER;
ALTER TABLE payments ADD COLUMN dues_retracted_at DATETIME;
