-- Public donations from non-members.
--
-- Until now `payments.member_id` was NOT NULL — every payment had to
-- belong to a member. With the public donation API (POST /public/donate),
-- a donor without an account hits the endpoint with just name + email +
-- amount, completes Stripe Checkout, and we record the payment.
--
-- Two cases:
--   1. The donor's email matches an existing member → member_id is set,
--      donor_name/donor_email are NULL. Donation appears in their
--      payment history page just like a logged-in donation.
--   2. No matching member → member_id is NULL, donor_name + donor_email
--      capture the giver's identity for accounting / receipts.
--
-- A CHECK constraint enforces the invariant: every payment has either
-- a member_id, or a (donor_name, donor_email) pair, but never neither.
--
-- SQLite can't ALTER COLUMN to relax NOT NULL, so this migration uses
-- the standard table-rewrite recipe.
--
-- We use `PRAGMA defer_foreign_keys = ON` rather than toggling
-- `foreign_keys` because the latter is inert inside a transaction (and
-- sqlx::migrate wraps every migration in one). With deferred checks,
-- FK violations during the rewrite are tolerated; SQLite verifies them
-- at COMMIT, by which time the rename has restored every reference.

PRAGMA defer_foreign_keys = ON;

CREATE TABLE payments_new (
    id TEXT PRIMARY KEY,
    -- Was NOT NULL — relaxed to allow public donations from non-members.
    member_id TEXT REFERENCES members(id),
    amount_cents INTEGER NOT NULL,
    currency TEXT NOT NULL DEFAULT 'USD',
    status TEXT NOT NULL CHECK(status IN ('Pending', 'Completed', 'Failed', 'Refunded')),
    payment_method TEXT NOT NULL CHECK(payment_method IN ('Stripe', 'Manual', 'Waived')),
    stripe_payment_id TEXT,
    description TEXT NOT NULL,
    paid_at DATETIME,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    -- Carried forward from migration 003
    payment_type TEXT NOT NULL DEFAULT 'membership',
    -- Carried forward from migration 013
    donation_campaign_id TEXT REFERENCES donation_campaigns(id),
    -- Carried forward from migration 014
    dues_extended_at DATETIME,
    -- New: identity for public donors (member_id IS NULL).
    donor_name TEXT,
    donor_email TEXT,
    -- Every payment has either a member or a public donor's identity.
    CHECK (
        member_id IS NOT NULL
        OR (donor_name IS NOT NULL AND donor_email IS NOT NULL)
    )
);

INSERT INTO payments_new (
    id, member_id, amount_cents, currency, status, payment_method,
    stripe_payment_id, description, paid_at, created_at, updated_at,
    payment_type, donation_campaign_id, dues_extended_at
)
SELECT
    id, member_id, amount_cents, currency, status, payment_method,
    stripe_payment_id, description, paid_at, created_at, updated_at,
    payment_type, donation_campaign_id, dues_extended_at
FROM payments;

DROP TABLE payments;
ALTER TABLE payments_new RENAME TO payments;

-- Recreate indexes that existed on the old table.
CREATE INDEX idx_payments_member ON payments(member_id);
CREATE INDEX idx_payments_status ON payments(status);
CREATE INDEX idx_payments_donation_campaign
    ON payments(donation_campaign_id)
    WHERE donation_campaign_id IS NOT NULL;
CREATE UNIQUE INDEX idx_payments_stripe_payment_id_unique
    ON payments(stripe_payment_id)
    WHERE stripe_payment_id IS NOT NULL;

-- New index: aggregating donor totals by email is the natural query
-- ("how much has Jane Doe given us this year?").
CREATE INDEX idx_payments_donor_email
    ON payments(donor_email)
    WHERE donor_email IS NOT NULL;
