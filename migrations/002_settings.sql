-- Application settings table for configurable options
CREATE TABLE IF NOT EXISTS app_settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    value_type TEXT NOT NULL CHECK(value_type IN ('string', 'number', 'boolean', 'json')),
    category TEXT NOT NULL,
    description TEXT,
    is_sensitive BOOLEAN NOT NULL DEFAULT 0,
    updated_by TEXT REFERENCES members(id),
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Insert default payment settings
INSERT INTO app_settings (key, value, value_type, category, description, is_sensitive) VALUES
    -- Payment Settings
    ('payment.regular_membership_fee', '5000', 'number', 'payment', 'Regular membership fee in cents', 0),
    ('payment.student_membership_fee', '2500', 'number', 'payment', 'Student membership fee in cents', 0),
    ('payment.corporate_membership_fee', '50000', 'number', 'payment', 'Corporate membership fee in cents', 0),
    ('payment.lifetime_membership_fee', '100000', 'number', 'payment', 'Lifetime membership fee in cents', 0),
    ('payment.grace_period_days', '30', 'number', 'payment', 'Days after expiration before suspension', 0),
    ('payment.reminder_days_before', '7', 'number', 'payment', 'Days before expiration to send reminder', 0),
    
    -- Membership Settings  
    ('membership.auto_approve', 'false', 'boolean', 'membership', 'Automatically approve new member signups', 0),
    ('membership.require_payment_for_activation', 'true', 'boolean', 'membership', 'Require payment before activating membership', 0),
    ('membership.default_duration_months', '12', 'number', 'membership', 'Default membership duration in months', 0),
    
    -- Organization Settings
    ('org.name', 'Coterie', 'string', 'organization', 'Organization name', 0),
    ('org.contact_email', 'admin@example.com', 'string', 'organization', 'Contact email address', 0),
    ('org.website_url', 'https://example.com', 'string', 'organization', 'Organization website URL', 0),
    
    -- Feature Flags
    ('features.events_enabled', 'true', 'boolean', 'features', 'Enable events module', 0),
    ('features.announcements_enabled', 'true', 'boolean', 'features', 'Enable announcements module', 0),
    ('features.member_directory_enabled', 'false', 'boolean', 'features', 'Enable public member directory', 0),
    ('features.blog_aggregation_enabled', 'false', 'boolean', 'features', 'Enable member blog aggregation', 0),
    
    -- Integration Settings (non-sensitive)
    ('integrations.discord.enabled', 'false', 'boolean', 'integrations', 'Enable Discord integration', 0),
    ('integrations.discord.guild_name', '', 'string', 'integrations', 'Discord server name', 0),
    ('integrations.unifi.enabled', 'false', 'boolean', 'integrations', 'Enable Unifi access integration', 0),
    ('integrations.stripe.enabled', 'false', 'boolean', 'integrations', 'Enable Stripe payments', 0),
    ('integrations.stripe.success_url', '/payment/success', 'string', 'integrations', 'Redirect URL after successful payment', 0),
    ('integrations.stripe.cancel_url', '/payment/cancel', 'string', 'integrations', 'Redirect URL after cancelled payment', 0);

-- Create index for faster lookups by category
CREATE INDEX idx_app_settings_category ON app_settings(category);

-- Audit table for settings changes
CREATE TABLE IF NOT EXISTS settings_audit (
    id TEXT PRIMARY KEY,
    setting_key TEXT NOT NULL,
    old_value TEXT,
    new_value TEXT NOT NULL,
    changed_by TEXT NOT NULL REFERENCES members(id),
    changed_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    reason TEXT
);

-- Create index for audit lookups
CREATE INDEX idx_settings_audit_key ON settings_audit(setting_key);
CREATE INDEX idx_settings_audit_changed_at ON settings_audit(changed_at);