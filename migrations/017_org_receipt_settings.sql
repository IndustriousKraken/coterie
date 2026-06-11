-- Two new org settings used by the member-facing receipt pages.
--
--   org.address — multi-line postal address. Shown on receipts as the
--                 letterhead. Optional; empty value just hides the
--                 line. Not used elsewhere yet.
--   org.tax_id  — tax ID / EIN. Some donors' accountants want it on
--                 the donation receipt. Optional, empty hides the
--                 line.
--
-- Inserts use INSERT OR IGNORE so re-applying against a DB that has
-- already been hand-edited (e.g. an early adopter who set these via
-- the admin settings UI before this migration shipped) is idempotent.

INSERT OR IGNORE INTO app_settings
    (key, value, value_type, category, description, is_sensitive)
VALUES
    ('org.address', '', 'string', 'organization',
     'Postal address shown on member receipts (multi-line OK).', 0),
    ('org.tax_id',  '', 'string', 'organization',
     'Tax ID / EIN shown on donation receipts. Leave blank if not applicable.', 0);
