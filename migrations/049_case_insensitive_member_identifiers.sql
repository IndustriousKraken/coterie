-- Email and username are unique without regard to case.
--
-- Lookups now compare with COLLATE NOCASE, so `relicix` finds `RelicIX`
-- and `mia@…` finds `Mia@…`. That is only safe if no two members can
-- differ by case alone — otherwise one login would answer to two
-- accounts. These indexes enforce it in the database, so every write
-- path is covered, and they are what the NOCASE lookups use.
--
-- If existing data already violates this, CREATE UNIQUE INDEX fails and
-- the migration stops. That is intended: reconciling two accounts is a
-- human decision, not something a migration should guess at. Stored
-- values are not modified.

CREATE UNIQUE INDEX idx_members_email_nocase ON members(email COLLATE NOCASE);
CREATE UNIQUE INDEX idx_members_username_nocase ON members(username COLLATE NOCASE);
