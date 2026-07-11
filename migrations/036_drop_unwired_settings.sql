-- Remove app_settings rows that no code has ever read. They render as
-- editable controls on the generic admin settings page (or sit invisibly
-- in unrendered categories) and let an operator "configure" behavior
-- that doesn't exist. Verified zero readers in src/ for every key below;
-- billing.max_retry_attempts is read (auto_renew) and is kept. Mirrors
-- the legacy-row cleanups in 029/030/035.
--
-- membership.auto_approve and membership.require_payment_for_activation
-- are superseded by membership.signup_mode (see the pay-at-signup
-- OpenSpec change): signup_mode=payment IS auto-approval-on-payment.
-- The features.* flags were aspirational module toggles never wired to
-- routing; re-add alongside a change that actually implements toggling.
DELETE FROM app_settings WHERE key IN (
    'membership.auto_approve',
    'membership.require_payment_for_activation',
    'membership.default_duration_months',
    'features.events_enabled',
    'features.announcements_enabled',
    'features.member_directory_enabled',
    'features.blog_aggregation_enabled',
    'billing.retry_interval_days',
    'billing.auto_renew_default',
    'billing.runner_interval_secs'
);
