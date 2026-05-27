-- Expense tracking: the flip side of Coterie's existing income
-- ledger (payments + donations). Three flat tables, no double-entry,
-- no journals.
--
--   expense_accounts    — named payment instruments (e.g., "Card 1 –
--                          Jane", "Petty Cash"). Operator-managed.
--   expense_categories  — operator-defined taxonomy (Supplies,
--                          Software, Insurance, …). Flat in v1.
--   expenses            — one row per outflow with spent_at,
--                          amount_cents, currency, description, FK
--                          to category + account, and an audit
--                          actor (`created_by`).
--
-- Indexes on `spent_at`, `category_id`, `account_id` keep the
-- monthly + per-category + per-account reports trivial — even a
-- high-activity org won't break a few thousand rows per year.
--
-- Soft-delete semantics live in the service, not here: deleting a
-- category or account referenced by an expense is refused; the
-- operator deactivates via `is_active = 0` instead.

CREATE TABLE IF NOT EXISTS expense_accounts (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    is_active INTEGER NOT NULL DEFAULT 1,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS expense_categories (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    slug TEXT NOT NULL UNIQUE,
    is_active INTEGER NOT NULL DEFAULT 1,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS expenses (
    id TEXT PRIMARY KEY,
    spent_at DATETIME NOT NULL,
    amount_cents INTEGER NOT NULL CHECK (amount_cents >= 0),
    currency TEXT NOT NULL DEFAULT 'USD',
    description TEXT NOT NULL,
    category_id TEXT NOT NULL REFERENCES expense_categories(id),
    account_id TEXT NOT NULL REFERENCES expense_accounts(id),
    notes TEXT,
    created_by TEXT NOT NULL REFERENCES members(id),
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_expenses_spent_at ON expenses(spent_at);
CREATE INDEX IF NOT EXISTS idx_expenses_category ON expenses(category_id);
CREATE INDEX IF NOT EXISTS idx_expenses_account ON expenses(account_id);
