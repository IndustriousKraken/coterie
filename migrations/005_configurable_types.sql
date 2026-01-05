-- Migration: 005_configurable_types
-- Description: Convert hardcoded type enums to database-driven configurable types
--
-- This migration:
-- 1. Creates new type tables (event_types, announcement_types, membership_types)
-- 2. Seeds default types
-- 3. Adds type_id columns to existing tables
-- 4. Migrates existing enum values to the new type_id references
-- 5. Creates indexes for performance

-- ============================================================================
-- PHASE 1: Create new type tables
-- ============================================================================

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
    is_system INTEGER NOT NULL DEFAULT 0,
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
    is_system INTEGER NOT NULL DEFAULT 0,
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
    is_system INTEGER NOT NULL DEFAULT 0,
    fee_cents INTEGER NOT NULL DEFAULT 0,
    billing_period TEXT NOT NULL DEFAULT 'yearly',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- ============================================================================
-- PHASE 2: Seed default types
-- ============================================================================

-- Default Event Types
INSERT OR IGNORE INTO event_types (id, name, slug, description, color, icon, sort_order, is_active, is_system, created_at, updated_at)
VALUES
    (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab',abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6))),
     'Meeting', 'meeting', NULL, '#2196F3', 'users', 0, 1, 1, datetime('now'), datetime('now')),
    (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab',abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6))),
     'Workshop', 'workshop', NULL, '#9C27B0', 'wrench', 1, 1, 1, datetime('now'), datetime('now')),
    (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab',abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6))),
     'CTF', 'ctf', NULL, '#F44336', 'flag', 2, 1, 1, datetime('now'), datetime('now')),
    (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab',abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6))),
     'Social', 'social', NULL, '#4CAF50', 'glass-cheers', 3, 1, 1, datetime('now'), datetime('now')),
    (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab',abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6))),
     'Training', 'training', NULL, '#FF9800', 'graduation-cap', 4, 1, 1, datetime('now'), datetime('now')),
    (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab',abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6))),
     'Hackathon', 'hackathon', NULL, '#E91E63', 'code', 5, 1, 1, datetime('now'), datetime('now'));

-- Default Announcement Types
INSERT OR IGNORE INTO announcement_types (id, name, slug, description, color, icon, sort_order, is_active, is_system, created_at, updated_at)
VALUES
    (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab',abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6))),
     'General', 'general', NULL, '#607D8B', NULL, 0, 1, 1, datetime('now'), datetime('now')),
    (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab',abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6))),
     'News', 'news', NULL, '#2196F3', NULL, 1, 1, 1, datetime('now'), datetime('now')),
    (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab',abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6))),
     'Achievement', 'achievement', NULL, '#FFC107', NULL, 2, 1, 1, datetime('now'), datetime('now')),
    (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab',abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6))),
     'Meeting', 'meeting', NULL, '#4CAF50', NULL, 3, 1, 1, datetime('now'), datetime('now')),
    (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab',abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6))),
     'CTF Result', 'ctf-result', NULL, '#F44336', NULL, 4, 1, 1, datetime('now'), datetime('now'));

-- Default Membership Types (with pricing from original PaymentConfig defaults)
INSERT OR IGNORE INTO membership_types (id, name, slug, description, color, icon, sort_order, is_active, is_system, fee_cents, billing_period, created_at, updated_at)
VALUES
    (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab',abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6))),
     'Regular', 'regular', NULL, '#2196F3', NULL, 0, 1, 1, 5000, 'yearly', datetime('now'), datetime('now')),
    (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab',abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6))),
     'Student', 'student', NULL, '#4CAF50', NULL, 1, 1, 1, 2500, 'yearly', datetime('now'), datetime('now')),
    (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab',abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6))),
     'Corporate', 'corporate', NULL, '#9C27B0', NULL, 2, 1, 1, 50000, 'yearly', datetime('now'), datetime('now')),
    (lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)),2) || '-' || substr('89ab',abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6))),
     'Lifetime', 'lifetime', NULL, '#FF9800', NULL, 3, 1, 1, 100000, 'lifetime', datetime('now'), datetime('now'));

-- ============================================================================
-- PHASE 3: Add type_id columns to existing tables
-- ============================================================================

-- Add event_type_id to events table
ALTER TABLE events ADD COLUMN event_type_id TEXT REFERENCES event_types(id);

-- Add announcement_type_id to announcements table
ALTER TABLE announcements ADD COLUMN announcement_type_id TEXT REFERENCES announcement_types(id);

-- Add membership_type_id to members table
ALTER TABLE members ADD COLUMN membership_type_id TEXT REFERENCES membership_types(id);

-- ============================================================================
-- PHASE 4: Migrate existing data
-- ============================================================================

-- Migrate event types (match on name)
UPDATE events
SET event_type_id = (
    SELECT id FROM event_types
    WHERE event_types.name = events.event_type
)
WHERE event_type_id IS NULL AND event_type IS NOT NULL;

-- Migrate announcement types (match on name, handle CTFResult -> CTF Result)
UPDATE announcements
SET announcement_type_id = (
    SELECT id FROM announcement_types
    WHERE announcement_types.name = announcements.announcement_type
       OR (announcements.announcement_type = 'CTFResult' AND announcement_types.slug = 'ctf-result')
)
WHERE announcement_type_id IS NULL AND announcement_type IS NOT NULL;

-- Migrate membership types (match on name)
UPDATE members
SET membership_type_id = (
    SELECT id FROM membership_types
    WHERE membership_types.name = members.membership_type
)
WHERE membership_type_id IS NULL AND membership_type IS NOT NULL;

-- ============================================================================
-- PHASE 5: Create indexes
-- ============================================================================

CREATE INDEX IF NOT EXISTS idx_event_types_slug ON event_types(slug);
CREATE INDEX IF NOT EXISTS idx_event_types_active ON event_types(is_active);

CREATE INDEX IF NOT EXISTS idx_announcement_types_slug ON announcement_types(slug);
CREATE INDEX IF NOT EXISTS idx_announcement_types_active ON announcement_types(is_active);

CREATE INDEX IF NOT EXISTS idx_membership_types_slug ON membership_types(slug);
CREATE INDEX IF NOT EXISTS idx_membership_types_active ON membership_types(is_active);

CREATE INDEX IF NOT EXISTS idx_events_type_id ON events(event_type_id);
CREATE INDEX IF NOT EXISTS idx_announcements_type_id ON announcements(announcement_type_id);
CREATE INDEX IF NOT EXISTS idx_members_type_id ON members(membership_type_id);
