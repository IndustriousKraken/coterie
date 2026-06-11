-- Coterie Initial Schema
-- Consolidated migration for member management system

-- =============================================================================
-- Core Tables
-- =============================================================================

-- Members table
CREATE TABLE IF NOT EXISTS members (
    id TEXT PRIMARY KEY,
    email TEXT UNIQUE NOT NULL,
    username TEXT UNIQUE NOT NULL,
    full_name TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('Pending', 'Active', 'Expired', 'Suspended', 'Honorary')),
    membership_type TEXT NOT NULL CHECK(membership_type IN ('Regular', 'Student', 'Corporate', 'Lifetime')),
    membership_type_id TEXT REFERENCES membership_types(id),
    joined_at DATETIME NOT NULL,
    expires_at DATETIME,
    dues_paid_until DATETIME,
    bypass_dues BOOLEAN NOT NULL DEFAULT 0,
    notes TEXT,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Member profiles (optional extended info)
CREATE TABLE IF NOT EXISTS member_profiles (
    member_id TEXT PRIMARY KEY REFERENCES members(id) ON DELETE CASCADE,
    bio TEXT,
    skills TEXT, -- JSON array stored as text
    show_in_directory BOOLEAN NOT NULL DEFAULT 0,
    blog_url TEXT,
    github_username TEXT,
    discord_id TEXT,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Events table
CREATE TABLE IF NOT EXISTS events (
    id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    event_type TEXT NOT NULL,
    event_type_id TEXT REFERENCES event_types(id),
    visibility TEXT NOT NULL DEFAULT 'MembersOnly' CHECK(visibility IN ('Public', 'MembersOnly', 'AdminOnly')),
    start_time DATETIME NOT NULL,
    end_time DATETIME,
    location TEXT,
    max_attendees INTEGER,
    rsvp_required BOOLEAN NOT NULL DEFAULT 0,
    created_by TEXT NOT NULL REFERENCES members(id),
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Event attendance
CREATE TABLE IF NOT EXISTS event_attendance (
    event_id TEXT NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    member_id TEXT NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK(status IN ('Registered', 'Waitlisted', 'Cancelled')),
    registered_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    attended BOOLEAN NOT NULL DEFAULT 0,
    PRIMARY KEY (event_id, member_id)
);

-- Announcements table
CREATE TABLE IF NOT EXISTS announcements (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    content TEXT NOT NULL,
    announcement_type TEXT NOT NULL,
    announcement_type_id TEXT REFERENCES announcement_types(id),
    is_public BOOLEAN NOT NULL DEFAULT 0,
    featured BOOLEAN NOT NULL DEFAULT 0,
    published_at DATETIME,
    created_by TEXT NOT NULL REFERENCES members(id),
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Payments table
CREATE TABLE IF NOT EXISTS payments (
    id TEXT PRIMARY KEY,
    member_id TEXT NOT NULL REFERENCES members(id),
    amount_cents INTEGER NOT NULL,
    currency TEXT NOT NULL DEFAULT 'USD',
    status TEXT NOT NULL CHECK(status IN ('Pending', 'Completed', 'Failed', 'Refunded')),
    payment_method TEXT NOT NULL CHECK(payment_method IN ('Stripe', 'Manual', 'Waived')),
    stripe_payment_id TEXT,
    description TEXT NOT NULL,
    paid_at DATETIME,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- =============================================================================
-- Authentication & Security
-- =============================================================================

-- Sessions table for auth
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    member_id TEXT NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    expires_at DATETIME NOT NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_used_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- CSRF tokens table
CREATE TABLE IF NOT EXISTS csrf_tokens (
    session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- API keys for integrations
CREATE TABLE IF NOT EXISTS api_keys (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    key_hash TEXT NOT NULL UNIQUE,
    permissions TEXT NOT NULL, -- JSON array stored as text
    last_used_at DATETIME,
    created_by TEXT NOT NULL REFERENCES members(id),
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at DATETIME
);

-- =============================================================================
-- Configurable Types
-- =============================================================================

-- Event Types
CREATE TABLE IF NOT EXISTS event_types (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    slug TEXT NOT NULL UNIQUE,
    description TEXT,
    color TEXT,
    icon TEXT,
    sort_order INTEGER NOT NULL DEFAULT 0,
    is_active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- Announcement Types
CREATE TABLE IF NOT EXISTS announcement_types (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    slug TEXT NOT NULL UNIQUE,
    description TEXT,
    color TEXT,
    icon TEXT,
    sort_order INTEGER NOT NULL DEFAULT 0,
    is_active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- Membership Types (includes pricing)
CREATE TABLE IF NOT EXISTS membership_types (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    slug TEXT NOT NULL UNIQUE,
    description TEXT,
    color TEXT,
    icon TEXT,
    sort_order INTEGER NOT NULL DEFAULT 0,
    is_active INTEGER NOT NULL DEFAULT 1,
    fee_cents INTEGER NOT NULL DEFAULT 0,
    billing_period TEXT NOT NULL DEFAULT 'yearly',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- =============================================================================
-- Application Settings
-- =============================================================================

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

-- Settings audit table
CREATE TABLE IF NOT EXISTS settings_audit (
    id TEXT PRIMARY KEY,
    setting_key TEXT NOT NULL,
    old_value TEXT,
    new_value TEXT NOT NULL,
    changed_by TEXT NOT NULL REFERENCES members(id),
    changed_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    reason TEXT
);

-- =============================================================================
-- Audit Log
-- =============================================================================

CREATE TABLE IF NOT EXISTS audit_logs (
    id TEXT PRIMARY KEY,
    actor_id TEXT REFERENCES members(id),
    action TEXT NOT NULL,
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    old_value TEXT, -- JSON stored as text
    new_value TEXT, -- JSON stored as text
    ip_address TEXT,
    user_agent TEXT,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- =============================================================================
-- Default Data: Types
-- =============================================================================

-- Default Event Types
INSERT OR IGNORE INTO event_types (id, name, slug, description, color, icon, sort_order, is_active, created_at, updated_at)
VALUES
    (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab',abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6))),
     'Member Meeting', 'member-meeting', 'Regular member meetings', '#2196F3', 'users', 0, 1, datetime('now'), datetime('now')),
    (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab',abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6))),
     'Social', 'social', 'Social gatherings and events', '#4CAF50', 'glass-cheers', 1, 1, datetime('now'), datetime('now'));

-- Default Announcement Types
INSERT OR IGNORE INTO announcement_types (id, name, slug, description, color, icon, sort_order, is_active, created_at, updated_at)
VALUES
    (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab',abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6))),
     'News', 'news', 'General news and updates', '#2196F3', NULL, 0, 1, datetime('now'), datetime('now')),
    (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab',abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6))),
     'Awards', 'awards', 'Member awards and recognition', '#FFC107', NULL, 1, 1, datetime('now'), datetime('now'));

-- Default Membership Types
INSERT OR IGNORE INTO membership_types (id, name, slug, description, color, icon, sort_order, is_active, fee_cents, billing_period, created_at, updated_at)
VALUES
    (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab',abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6))),
     'Member', 'member', 'Standard membership', '#2196F3', NULL, 0, 1, 500, 'monthly', datetime('now'), datetime('now')),
    (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab',abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6))),
     'Associate', 'associate', 'Associate membership', '#9C27B0', NULL, 1, 1, 10000, 'monthly', datetime('now'), datetime('now')),
    (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab',abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6))),
     'Life Member', 'life-member', 'Lifetime membership', '#FF9800', NULL, 2, 1, 1000000, 'lifetime', datetime('now'), datetime('now'));

-- =============================================================================
-- Default Data: Settings
-- =============================================================================

INSERT INTO app_settings (key, value, value_type, category, description, is_sensitive) VALUES
    -- Membership Settings
    ('membership.auto_approve', 'false', 'boolean', 'membership', 'Automatically approve new member signups', 0),
    ('membership.require_payment_for_activation', 'true', 'boolean', 'membership', 'Require payment before activating membership', 0),
    ('membership.default_duration_months', '12', 'number', 'membership', 'Default membership duration in months', 0),
    ('membership.grace_period_days', '30', 'number', 'membership', 'Days after expiration before suspension', 0),
    ('membership.reminder_days_before', '7', 'number', 'membership', 'Days before expiration to send reminder', 0),

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

-- =============================================================================
-- Indexes
-- =============================================================================

-- Members
CREATE INDEX idx_members_email ON members(email);
CREATE INDEX idx_members_username ON members(username);
CREATE INDEX idx_members_status ON members(status);
CREATE INDEX idx_members_type_id ON members(membership_type_id);

-- Events
CREATE INDEX idx_events_start_time ON events(start_time);
CREATE INDEX idx_events_event_type ON events(event_type);
CREATE INDEX idx_events_visibility ON events(visibility);
CREATE INDEX idx_events_type_id ON events(event_type_id);

-- Announcements
CREATE INDEX idx_announcements_published ON announcements(published_at);
CREATE INDEX idx_announcements_public ON announcements(is_public);
CREATE INDEX idx_announcements_type_id ON announcements(announcement_type_id);

-- Payments
CREATE INDEX idx_payments_member ON payments(member_id);
CREATE INDEX idx_payments_status ON payments(status);

-- Sessions
CREATE INDEX idx_sessions_token ON sessions(token_hash);
CREATE INDEX idx_sessions_expires ON sessions(expires_at);
CREATE INDEX idx_csrf_tokens_session ON csrf_tokens(session_id);

-- Audit
CREATE INDEX idx_audit_logs_actor ON audit_logs(actor_id);
CREATE INDEX idx_audit_logs_entity ON audit_logs(entity_type, entity_id);

-- Settings
CREATE INDEX idx_app_settings_category ON app_settings(category);
CREATE INDEX idx_settings_audit_key ON settings_audit(setting_key);
CREATE INDEX idx_settings_audit_changed_at ON settings_audit(changed_at);

-- Types
CREATE INDEX IF NOT EXISTS idx_event_types_slug ON event_types(slug);
CREATE INDEX IF NOT EXISTS idx_event_types_active ON event_types(is_active);
CREATE INDEX IF NOT EXISTS idx_announcement_types_slug ON announcement_types(slug);
CREATE INDEX IF NOT EXISTS idx_announcement_types_active ON announcement_types(is_active);
CREATE INDEX IF NOT EXISTS idx_membership_types_slug ON membership_types(slug);
CREATE INDEX IF NOT EXISTS idx_membership_types_active ON membership_types(is_active);
