-- Email configuration moves from env vars to DB-backed settings so
-- admins can troubleshoot deliverability without shelling into the
-- server. Default mode is "log" — emails are written to tracing logs
-- only until the admin configures SMTP in the UI.
--
-- The SMTP password is encrypted at rest using session_secret as the
-- key; see src/auth/secret_crypto.rs. Other fields are plaintext.

INSERT INTO app_settings (key, value, value_type, category, description, is_sensitive) VALUES
    -- Send mode: "log" writes to tracing; "smtp" actually sends.
    ('email.mode', 'log', 'string', 'email',
     'Send mode: log (tracing only, for dev) or smtp (real delivery)', 0),

    -- Envelope fields
    ('email.from_address', 'noreply@localhost', 'string', 'email',
     'From address on outbound mail', 0),
    ('email.from_name', 'Coterie', 'string', 'email',
     'Display name paired with From address', 0),

    -- SMTP connection (ignored in log mode)
    ('email.smtp_host', '', 'string', 'email',
     'SMTP server hostname (e.g. smtp.postmarkapp.com)', 0),
    ('email.smtp_port', '587', 'number', 'email',
     'SMTP port (587 for STARTTLS, 465 for implicit TLS)', 0),
    ('email.smtp_username', '', 'string', 'email',
     'SMTP username', 0),
    ('email.smtp_password', '', 'string', 'email',
     'SMTP password (encrypted at rest)', 1),

    -- Last test result — shown in the admin UI so operators can see
    -- configuration health at a glance. Updated on every test-email
    -- attempt.
    ('email.last_test_at', '', 'string', 'email',
     'When the last test email was attempted (ISO 8601, empty if never)', 0),
    ('email.last_test_ok', 'false', 'boolean', 'email',
     'Whether the last test email succeeded', 0),
    ('email.last_test_error', '', 'string', 'email',
     'Error from the last test email (empty on success)', 0);
