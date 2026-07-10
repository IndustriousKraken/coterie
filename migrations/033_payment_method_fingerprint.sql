-- Card fingerprint for de-duplicating saved cards.
--
-- The Stripe payment-history/card backfill mirrors cards already
-- attached to a member's Stripe customer into `payment_methods`. Stripe
-- assigns the same `fingerprint` to the same underlying card even when
-- it is attached as several distinct `pm_*` ids, so the backfill keys
-- de-dup on the fingerprint rather than the pm id — re-running (or a
-- card re-attached under a new pm id) does not create a second row.
--
-- Nullable: pre-existing rows and interactively-added cards (SetupIntent
-- flow) may not have a fingerprint recorded; de-dup only fires when both
-- the incoming card and an existing row carry the same non-null value.

ALTER TABLE payment_methods ADD COLUMN card_fingerprint TEXT;
