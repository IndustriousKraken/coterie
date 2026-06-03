-- a40: opt-out toggle for the daily "is a newer stable release
-- available?" check. Default on, matching the security-sensitive
-- posture of keeping the instance current. When enabled the instance
-- makes at most one unauthenticated GET to api.github.com per day; the
-- result feeds an admin-only "update available" banner. The render path
-- never contacts GitHub — only the background task does. Turning this
-- off stops the fetch and hides the banner regardless of any value the
-- task previously cached.

INSERT INTO app_settings (key, value, value_type, category, description, is_sensitive) VALUES
    ('updates.check_enabled', 'true', 'boolean', 'updates',
     'Check GitHub daily for a newer stable release and show admins an update banner. Enabling contacts the public GitHub releases API (unauthenticated).', 0);
