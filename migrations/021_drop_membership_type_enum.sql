-- Drop the legacy `membership_type` enum column on `members`.
--
-- Until now the schema carried two parallel paths: a fixed enum
-- (`membership_type` TEXT CHECK IN (Regular,Student,Corporate,Lifetime))
-- and an FK (`membership_type_id` TEXT REFERENCES membership_types).
-- Signup wrote the enum, billing read the FK, and new rows landed
-- with `membership_type_id IS NULL` — every billing flow then had a
-- fallback path emitting an admin alert. Worse, the seeded
-- `membership_types` rows have slugs (`member`, `associate`,
-- `life-member`) that don't even map to the enum's hardcoded names,
-- so no slug-based lookup could rescue the orphans.
--
-- This migration finishes what was started:
--   1. Backfill `membership_type_id` on any row where it's NULL,
--      pointing at the first `is_active` row in `membership_types`
--      ordered by `sort_order`. The previous enum string is discarded
--      — it had no consumer worth preserving.
--   2. Recreate the table without `membership_type`, with
--      `membership_type_id` NOT NULL and the FK in place. SQLite
--      can't ALTER COLUMN to drop a column or relax NULL on this
--      version, so this is the table-rewrite recipe (same shape as
--      migration 016).
--
-- After this migration the domain `MembershipType` enum is removed
-- and signup writes `membership_type_id` directly.

PRAGMA defer_foreign_keys = ON;

-- 1. Backfill any NULL membership_type_id rows.
UPDATE members
SET membership_type_id = (
    SELECT id FROM membership_types
    WHERE is_active = 1
    ORDER BY sort_order ASC, name ASC
    LIMIT 1
)
WHERE membership_type_id IS NULL;

-- Defensive: if no active membership_types row exists at all, leave
-- the column NULL on those rows. The CREATE TABLE below will fail
-- with NOT NULL violation, which is the correct loud signal — an org
-- that wiped all membership types and has members can't proceed
-- without first restoring at least one type.

CREATE TABLE members_new (
    id TEXT PRIMARY KEY,
    email TEXT UNIQUE NOT NULL,
    username TEXT UNIQUE NOT NULL,
    full_name TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('Pending', 'Active', 'Expired', 'Suspended', 'Honorary')),
    membership_type_id TEXT NOT NULL REFERENCES membership_types(id),
    joined_at DATETIME NOT NULL,
    expires_at DATETIME,
    dues_paid_until DATETIME,
    bypass_dues BOOLEAN NOT NULL DEFAULT 0,
    notes TEXT,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    -- Carried forward from migration 003
    stripe_customer_id TEXT,
    stripe_subscription_id TEXT,
    billing_mode TEXT NOT NULL DEFAULT 'manual',
    -- Carried forward from migration 005
    is_admin BOOLEAN NOT NULL DEFAULT 0,
    -- Carried forward from migration 007
    email_verified_at DATETIME,
    -- Carried forward from migration 009
    dues_reminder_sent_at DATETIME,
    -- Carried forward from migration 012
    discord_id TEXT,
    -- Carried forward from migration 018
    totp_secret_encrypted TEXT,
    totp_enabled_at DATETIME,
    totp_recovery_codes TEXT
);

INSERT INTO members_new (
    id, email, username, full_name, password_hash, status,
    membership_type_id, joined_at, expires_at, dues_paid_until,
    bypass_dues, notes, created_at, updated_at,
    stripe_customer_id, stripe_subscription_id, billing_mode,
    is_admin, email_verified_at, dues_reminder_sent_at, discord_id,
    totp_secret_encrypted, totp_enabled_at, totp_recovery_codes
)
SELECT
    id, email, username, full_name, password_hash, status,
    membership_type_id, joined_at, expires_at, dues_paid_until,
    bypass_dues, notes, created_at, updated_at,
    stripe_customer_id, stripe_subscription_id, billing_mode,
    is_admin, email_verified_at, dues_reminder_sent_at, discord_id,
    totp_secret_encrypted, totp_enabled_at, totp_recovery_codes
FROM members;

DROP TABLE members;
ALTER TABLE members_new RENAME TO members;

-- Recreate indexes that existed on the old table.
CREATE INDEX idx_members_email ON members(email);
CREATE INDEX idx_members_username ON members(username);
CREATE INDEX idx_members_status ON members(status);
CREATE INDEX idx_members_type_id ON members(membership_type_id);
CREATE INDEX idx_members_is_admin ON members(is_admin) WHERE is_admin = 1;
