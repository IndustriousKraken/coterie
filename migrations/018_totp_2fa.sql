-- Admin / member TOTP two-factor authentication.
--
-- Three new columns on `members`:
--   totp_secret_encrypted   The TOTP shared secret, encrypted at rest
--                           with the same SecretCrypto used for SMTP /
--                           Discord secrets. NULL when 2FA is off.
--                           Stealing the DB on its own does not give
--                           an attacker the ability to mint TOTP codes.
--   totp_enabled_at         When the member completed enrollment.
--                           NULL = 2FA is off (login skips the second
--                           step). Non-null = login MUST go through the
--                           TOTP step.
--   totp_recovery_codes     JSON array of argon2-hashed recovery codes.
--                           One-time use; consuming a code rewrites the
--                           array without it. Hashed (not stored
--                           plaintext) so a DB leak doesn't grant
--                           account access via the recovery path.
--                           Generated at enrollment, regeneratable from
--                           the security page.
--
-- Plus a `pending_logins` table that holds the short-lived (5-minute)
-- intermediate token between password verification and TOTP
-- verification. Distinct from `sessions` so a half-finished login can
-- never be mistaken for an authenticated session — the `require_auth`
-- middleware reads only `sessions`.

-- TEXT (not BLOB) because SecretCrypto returns base64-encoded ciphertext,
-- consistent with how SMTP passwords are stored in app_settings.
ALTER TABLE members ADD COLUMN totp_secret_encrypted TEXT;
ALTER TABLE members ADD COLUMN totp_enabled_at DATETIME;
ALTER TABLE members ADD COLUMN totp_recovery_codes TEXT;

CREATE TABLE IF NOT EXISTS pending_logins (
    id TEXT PRIMARY KEY,
    member_id TEXT NOT NULL,
    -- SHA256 of the raw token. Same scheme as `sessions.token_hash`:
    -- the cookie carries the raw token; the DB only ever sees the
    -- hash, so a DB leak doesn't let an attacker mint a real session
    -- by completing the second step.
    token_hash TEXT NOT NULL UNIQUE,
    -- Whether the original /login form had remember-me checked. The
    -- final session (created after TOTP verifies) inherits this so
    -- the user gets the same long-lived cookie they would have
    -- without 2FA in the way.
    remember_me INTEGER NOT NULL DEFAULT 0,
    expires_at DATETIME NOT NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (member_id) REFERENCES members(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_pending_logins_token
    ON pending_logins(token_hash);
CREATE INDEX IF NOT EXISTS idx_pending_logins_expires
    ON pending_logins(expires_at);
