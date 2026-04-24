-- Add explicit is_admin column to members.
--
-- Previously admin status was determined by the string "ADMIN" appearing in
-- members.notes. Because notes is self-editable via the member profile,
-- that allowed self-promotion. This migration introduces a dedicated column
-- and backfills it from the legacy marker.

ALTER TABLE members ADD COLUMN is_admin BOOLEAN NOT NULL DEFAULT 0;

UPDATE members
SET is_admin = 1
WHERE notes LIKE '%ADMIN%';

CREATE INDEX idx_members_is_admin ON members(is_admin) WHERE is_admin = 1;
