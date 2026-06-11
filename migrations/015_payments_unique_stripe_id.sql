-- Defensive uniqueness on stripe_payment_id. Without this, a code
-- bug or unusual sequence (e.g. cs_→pi_ upgrade colliding with a
-- separate row that already has the same pi_) could leave two
-- Payment rows referencing the same Stripe charge — find_by_stripe_id
-- returns at-most-one (LIMIT 1, ordering undefined) and refunds
-- routed by Stripe ID could mark the wrong row.
--
-- Partial index: only enforces uniqueness when stripe_payment_id IS
-- NOT NULL, since manual / waived payments legitimately have NULL
-- here and we don't want to collapse them all to a single row.

CREATE UNIQUE INDEX IF NOT EXISTS idx_payments_stripe_payment_id_unique
    ON payments(stripe_payment_id)
    WHERE stripe_payment_id IS NOT NULL;
