-- Members table
CREATE TABLE IF NOT EXISTS members (
    id TEXT PRIMARY KEY,
    email TEXT UNIQUE NOT NULL,
    username TEXT UNIQUE NOT NULL,
    full_name TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('Pending', 'Active', 'Expired', 'Suspended', 'Honorary')),
    membership_type TEXT NOT NULL CHECK(membership_type IN ('Regular', 'Student', 'Corporate', 'Lifetime')),
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
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    event_type TEXT NOT NULL CHECK(event_type IN ('Meeting', 'Workshop', 'CTF', 'Social', 'Training')),
    visibility TEXT NOT NULL CHECK(visibility IN ('Public', 'MembersOnly', 'AdminOnly')),
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
    announcement_type TEXT NOT NULL CHECK(announcement_type IN ('News', 'Achievement', 'Meeting', 'CTFResult', 'General')),
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

-- Audit log table
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

-- Sessions table for auth
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    member_id TEXT NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    expires_at DATETIME NOT NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_used_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
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

-- Create indexes for better query performance
CREATE INDEX idx_members_email ON members(email);
CREATE INDEX idx_members_username ON members(username);
CREATE INDEX idx_members_status ON members(status);
CREATE INDEX idx_events_start_time ON events(start_time);
CREATE INDEX idx_events_visibility ON events(visibility);
CREATE INDEX idx_announcements_published ON announcements(published_at);
CREATE INDEX idx_announcements_public ON announcements(is_public);
CREATE INDEX idx_payments_member ON payments(member_id);
CREATE INDEX idx_payments_status ON payments(status);
CREATE INDEX idx_sessions_token ON sessions(token_hash);
CREATE INDEX idx_sessions_expires ON sessions(expires_at);
CREATE INDEX idx_audit_logs_actor ON audit_logs(actor_id);
CREATE INDEX idx_audit_logs_entity ON audit_logs(entity_type, entity_id);