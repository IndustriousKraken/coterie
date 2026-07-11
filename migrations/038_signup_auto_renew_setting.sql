-- Pay-at-signup auto-renew enrollment (see the pay-at-signup OpenSpec
-- change). When true (default) and signup_mode=payment, the signup
-- checkout saves the paying card for off-session use and the completed
-- payment enrolls the member in Coterie-managed auto-renew with the
-- next renewal scheduled. Set false for one-off signup charges.
INSERT INTO app_settings (key, value, value_type, category, description, is_sensitive) VALUES
    ('membership.signup_auto_renew', 'true', 'boolean', 'membership',
     'Enroll paying signups in auto-renew (saves their card; next renewal scheduled automatically)', 0);
