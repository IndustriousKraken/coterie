-- Billing-specific settings for the recurring billing system
INSERT OR IGNORE INTO app_settings (key, value, value_type, category, description, is_sensitive) VALUES
    ('billing.max_retry_attempts', '3', 'number', 'billing', 'Maximum charge retry attempts before marking as failed', 0),
    ('billing.retry_interval_days', '3', 'number', 'billing', 'Days between charge retry attempts', 0),
    ('billing.auto_renew_default', 'true', 'boolean', 'billing', 'Enable auto-renewal by default for new members', 0),
    ('billing.runner_interval_secs', '3600', 'number', 'billing', 'Billing runner check interval in seconds (default: 1 hour)', 0);
