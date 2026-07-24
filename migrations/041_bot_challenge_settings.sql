-- Bot-challenge (Turnstile) config moves from env vars to DB-backed
-- settings so admins can enable the captcha and set/rotate the secret
-- from the portal without shelling into the server and restarting —
-- matching the Stripe/Discord/UniFi pattern (029/012/030). The secret
-- key is encrypted at rest via SecretCrypto; the site key is public
-- (the marketing join form drives the widget from its own config) and
-- stored plaintext for admin reference.
--
-- No env carry-over: bot-challenge was wired entirely from env and its
-- values were never in the DB. The admin fills these in.
INSERT INTO app_settings (key, value, value_type, category, description, is_sensitive) VALUES
    ('bot_challenge.provider', 'disabled', 'string', 'bot_challenge',
     'Bot-challenge provider: disabled (no captcha) or turnstile', 0),
    ('bot_challenge.secret_key', '', 'string', 'bot_challenge',
     'Turnstile secret key. Encrypted at rest; used only to call siteverify.', 1),
    ('bot_challenge.site_key', '', 'string', 'bot_challenge',
     'Turnstile public site key. Admin reference — the marketing widget uses it.', 0),
    ('bot_challenge.timeout_ms', '5000', 'number', 'bot_challenge',
     'Per-call timeout (ms) for the provider siteverify request', 0);
