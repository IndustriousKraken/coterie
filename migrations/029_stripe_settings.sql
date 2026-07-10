-- Stripe configuration moves from env vars to DB-backed settings so
-- admins can add/rotate keys from the portal without shelling into the
-- server and restarting — matching the email/discord pattern. The
-- secret key and webhook signing secret are encrypted at rest using
-- SecretCrypto (key derived from session_secret); the publishable key
-- is public and stored plaintext.

-- First, drop the dead `integrations.stripe.*` rows. They were seeded
-- by 001_initial_schema but read NOWHERE in the code — Stripe was
-- wired entirely from env. They made the generic settings page show a
-- false "Stripe: disabled" toggle while Stripe ran from `.env`. No
-- data carry-over: they never drove anything.
DELETE FROM app_settings WHERE key IN (
    'integrations.stripe.enabled',
    'integrations.stripe.success_url',
    'integrations.stripe.cancel_url'
);

INSERT INTO app_settings (key, value, value_type, category, description, is_sensitive) VALUES
    ('stripe.enabled', 'false', 'boolean', 'stripe',
     'Enable Stripe payment processing', 0),

    -- Keys. publishable is public (Stripe.js); the other two are secret.
    ('stripe.publishable_key', '', 'string', 'stripe',
     'Stripe publishable key (pk_...). Public — used by Stripe.js.', 0),
    ('stripe.secret_key', '', 'string', 'stripe',
     'Stripe secret key (sk_...). Encrypted at rest.', 1),
    ('stripe.webhook_secret', '', 'string', 'stripe',
     'Stripe webhook signing secret (whsec_...). Encrypted at rest.', 1),

    -- Optional redirect targets after Checkout.
    ('stripe.success_url', '/payment/success', 'string', 'stripe',
     'Redirect path after a successful payment', 0),
    ('stripe.cancel_url', '/payment/cancel', 'string', 'stripe',
     'Redirect path after a cancelled payment', 0),

    -- Connection-test status display (mirrors discord/email).
    ('stripe.last_test_at', '', 'string', 'stripe',
     'When the last Stripe test was attempted (ISO 8601, empty if never)', 0),
    ('stripe.last_test_ok', 'false', 'boolean', 'stripe',
     'Whether the last Stripe test succeeded', 0),
    ('stripe.last_test_error', '', 'string', 'stripe',
     'Error from the last Stripe test (empty on success)', 0);
