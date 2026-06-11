-- Email verification and password reset infrastructure.

-- Track when each member verified ownership of their email address.
-- NULL means never verified. Existing members get backfilled to their
-- joined_at (they were already accepted into the system so we treat
-- them as implicitly verified; otherwise import/migration would force
-- every legacy member to re-verify).
ALTER TABLE members ADD COLUMN email_verified_at DATETIME;

UPDATE members SET email_verified_at = joined_at WHERE email_verified_at IS NULL;

-- Email verification tokens. Single-use, short TTL. Token itself is
-- hashed (same treatment as sessions/CSRF): the plaintext only exists
-- in the emailed link.
CREATE TABLE email_verification_tokens (
    id           TEXT PRIMARY KEY,
    member_id    TEXT NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    token_hash   TEXT NOT NULL UNIQUE,
    created_at   DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at   DATETIME NOT NULL,
    consumed_at  DATETIME
);

CREATE INDEX idx_email_verification_tokens_member ON email_verification_tokens(member_id);
CREATE INDEX idx_email_verification_tokens_expires ON email_verification_tokens(expires_at);

-- Password reset tokens — same shape as verification tokens but with
-- shorter TTL (1 hour) and different purpose.
CREATE TABLE password_reset_tokens (
    id           TEXT PRIMARY KEY,
    member_id    TEXT NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    token_hash   TEXT NOT NULL UNIQUE,
    created_at   DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at   DATETIME NOT NULL,
    consumed_at  DATETIME
);

CREATE INDEX idx_password_reset_tokens_member ON password_reset_tokens(member_id);
CREATE INDEX idx_password_reset_tokens_expires ON password_reset_tokens(expires_at);
