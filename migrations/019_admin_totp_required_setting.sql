-- Toggle that gates admin pages behind TOTP enrollment.
--
-- When 'true', any admin without `members.totp_enabled_at` set is
-- redirected from /portal/admin/* to /portal/profile/security with a
-- message explaining they must enroll. Default 'false' so existing
-- deployments aren't disrupted on upgrade — operators flip it once
-- their admin team has enrolled.
--
-- Lives in a new 'auth' category. The settings UI gains the category
-- automatically once it's listed in `fetch_settings_by_category`.
--
-- INSERT OR IGNORE so re-applying against a hand-edited DB stays
-- idempotent.

INSERT OR IGNORE INTO app_settings
    (key, value, value_type, category, description, is_sensitive)
VALUES
    ('auth.require_totp_for_admins', 'false', 'boolean', 'auth',
     'Require admins to enroll in two-factor authentication before \
      accessing admin pages. Members can still sign in normally.',
     0);
