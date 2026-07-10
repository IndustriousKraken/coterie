-- UniFi configuration moves from env vars to DB-backed settings so admins
-- can add / rotate controller credentials from the portal without shelling
-- into the server and restarting — matching the email/discord/stripe
-- pattern. The controller password is encrypted at rest using SecretCrypto
-- (key derived from session_secret); everything else is stored plaintext.

-- First, drop the dead `integrations.unifi.enabled` row. It was seeded by
-- 001_initial_schema but read NOWHERE in the code — UniFi was wired
-- entirely from env. It made the generic settings page show a false
-- "Unifi: disabled" toggle while UniFi ran from `.env`. No data carry-over:
-- it never drove anything.
DELETE FROM app_settings WHERE key = 'integrations.unifi.enabled';

INSERT INTO app_settings (key, value, value_type, category, description, is_sensitive) VALUES
    ('unifi.enabled', 'false', 'boolean', 'unifi',
     'Enable UniFi Access door integration', 0),

    -- Controller connection.
    ('unifi.controller_url', '', 'string', 'unifi',
     'UniFi controller base URL (e.g. https://192.168.1.1)', 0),
    ('unifi.username', '', 'string', 'unifi',
     'UniFi controller admin username', 0),
    ('unifi.password', '', 'string', 'unifi',
     'UniFi controller admin password. Encrypted at rest.', 1),
    ('unifi.site_id', 'default', 'string', 'unifi',
     'UniFi site identifier (usually "default")', 0),

    -- Connection-test status display (mirrors discord/stripe/email).
    ('unifi.last_test_at', '', 'string', 'unifi',
     'When the last UniFi test was attempted (ISO 8601, empty if never)', 0),
    ('unifi.last_test_ok', 'false', 'boolean', 'unifi',
     'Whether the last UniFi test succeeded', 0),
    ('unifi.last_test_error', '', 'string', 'unifi',
     'Error from the last UniFi test (empty on success)', 0);
