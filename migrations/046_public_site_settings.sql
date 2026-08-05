-- Companion public-site change notifications.
--
-- An organization's public website renders Coterie's public events and
-- announcements. Until now it could only learn of a change by polling,
-- which puts a floor on how wrong it can be — and a stale retraction is
-- a disclosure, not an annoyance.
--
-- Both settings default empty: with no endpoint the capability is
-- entirely inert, so every deployment without a companion site is
-- unaffected by its existence. The secret is encrypted at rest via
-- SecretCrypto and masked in the admin UI, matching the Stripe /
-- Discord / UniFi / bot-challenge credential pattern.
INSERT INTO app_settings (key, value, value_type, category, description, is_sensitive) VALUES
    ('public_site.endpoint_url', '', 'string', 'public_site',
     'Endpoint on your public website that change notifications are POSTed to. Must start with http:// or https://; leave empty to send nothing.', 0),
    ('public_site.secret', '', 'string', 'public_site',
     'Shared secret used to sign notifications (X-Coterie-Signature: sha256=HMAC of the body). Encrypted at rest; the receiver must verify it.', 1);
