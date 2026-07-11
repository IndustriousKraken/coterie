-- Signup funnel mode (see the pay-at-signup OpenSpec change).
--   approval — signup creates a Pending member; an admin activates.
--   payment  — signup returns a Stripe Checkout URL; a completed
--              membership payment activates the member.
-- Supersedes the never-wired membership.auto_approve /
-- membership.require_payment_for_activation rows removed by 036:
-- signup_mode=payment IS auto-approval-on-payment.
INSERT INTO app_settings (key, value, value_type, category, description, is_sensitive) VALUES
    ('membership.signup_mode', 'approval', 'string', 'membership',
     'Signup funnel: approval (admin activates new members) or payment (paid Stripe checkout at signup activates automatically)', 0);
