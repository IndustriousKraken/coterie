-- Add Stripe billing fields to members
ALTER TABLE members ADD COLUMN stripe_customer_id TEXT;
ALTER TABLE members ADD COLUMN stripe_subscription_id TEXT;
ALTER TABLE members ADD COLUMN billing_mode TEXT NOT NULL DEFAULT 'manual';
-- billing_mode values: 'manual', 'coterie_managed', 'stripe_subscription'

-- Saved payment methods (cards)
CREATE TABLE payment_methods (
    id TEXT PRIMARY KEY,
    member_id TEXT NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    stripe_payment_method_id TEXT NOT NULL UNIQUE,
    card_last_four TEXT NOT NULL,
    card_brand TEXT NOT NULL,
    exp_month INTEGER NOT NULL,
    exp_year INTEGER NOT NULL,
    is_default INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_payment_methods_member ON payment_methods(member_id);

-- Scheduled payments for Coterie-managed billing
CREATE TABLE scheduled_payments (
    id TEXT PRIMARY KEY,
    member_id TEXT NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    membership_type_id TEXT NOT NULL,
    amount_cents INTEGER NOT NULL,
    currency TEXT NOT NULL DEFAULT 'USD',
    due_date TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    -- status values: 'pending', 'processing', 'completed', 'failed', 'canceled'
    retry_count INTEGER NOT NULL DEFAULT 0,
    last_attempt_at TEXT,
    payment_id TEXT REFERENCES payments(id),
    failure_reason TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_scheduled_payments_due ON scheduled_payments(due_date, status);
CREATE INDEX idx_scheduled_payments_member ON scheduled_payments(member_id);

-- Add payment_type to payments table
ALTER TABLE payments ADD COLUMN payment_type TEXT NOT NULL DEFAULT 'membership';
-- payment_type values: 'membership', 'donation', 'other'

-- Donation campaigns (optional fundraising targets)
CREATE TABLE donation_campaigns (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    slug TEXT NOT NULL UNIQUE,
    description TEXT,
    goal_cents INTEGER,
    is_active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_donation_campaigns_slug ON donation_campaigns(slug);
