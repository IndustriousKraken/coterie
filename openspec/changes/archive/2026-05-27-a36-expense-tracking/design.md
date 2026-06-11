## Context

Coterie's existing "money" surfaces:
- `payments` — captures incoming dues, donations, manual one-offs, Stripe-initiated charges. Has a rich `Payment` domain type with `Payer`, `PaymentKind`, `StripeRef` sum types.
- `donation_campaigns` + `donations` — campaign-attributable contributions.
- Three payment-recording entry points (per `payment-recording` capability): `PaymentService::record_manual`, `WebhookDispatcher::handle_*`, `BillingService::process_scheduled_payment`.

Expenses are the missing flip side. Today, operators track them in spreadsheets and reconcile against bank statements by hand. The reconciliation pain is real and recurring (monthly + annually).

The design here is deliberately small: model expenses, categories, and accounts as three plain CRUD surfaces; offer a couple of reconciliation reports that combine expense data with the existing payments/donations data; export to CSV for tax season. No accounting rules, no double-entry, no journal/ledger abstractions. If the org outgrows this, it grows into QuickBooks or Wave or similar; Coterie's job is to be the source of truth for "what we did," not to replace specialized accounting software.

## Goals / Non-Goals

**Goals:**
- Operators can record an expense in under 30 seconds: date, amount, category, account, description. Form is small.
- Monthly reconciliation: per-account totals visible in one page. The operator should be able to look at a debit-card statement, click the matching month in Coterie, and verify Coterie's per-account total matches the statement's total.
- Annual report: revenue (payments + donations) minus expenses, broken by category. Single page, single year.
- Tax-prep export: every transaction in a single CSV, sorted by date, with the columns an accountant expects.
- Audit trail: every expense create/update/delete writes an audit row, same way member-mutation does.

**Non-Goals:**
- Tax-rule application. The operator interprets the report, not Coterie.
- Public transparency dashboard. Some orgs publish their expense data; that's a separate access-control + redaction concern.
- Double-entry bookkeeping, journals, multi-period accruals. Cash-basis only.
- Receipt image/PDF storage. Defer.
- Bank statement OFX/CSV import. Defer.
- Multi-currency. Single currency per the existing payment-side convention.
- Sub-categories / hierarchical categories. Flat list in v1.

## Decisions

### D1. Three tables, all simple

```sql
CREATE TABLE expense_accounts (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    is_active INTEGER NOT NULL DEFAULT 1,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE expense_categories (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    slug TEXT NOT NULL UNIQUE,
    is_active INTEGER NOT NULL DEFAULT 1,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE expenses (
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

CREATE INDEX idx_expenses_spent_at ON expenses(spent_at);
CREATE INDEX idx_expenses_category ON expenses(category_id);
CREATE INDEX idx_expenses_account ON expenses(account_id);
```

`amount_cents >= 0`: expenses are always positive numbers. Refunds (which would be negative) go through the existing `payments` table via the Stripe refund flow.

`spent_at` is when the expense happened (date on the receipt), not when the row was created. Reconciliation needs the spent date.

### D2. Account model: named instruments

An `ExpenseAccount` is a payment instrument: "Debit Card 1 – Jane", "Debit Card 2 – Bob", "Petty Cash", "Org PayPal Balance." It's NOT an accounting "account" (no balance, no liability/asset classification). The operator creates whatever set matches their physical payment instruments.

The two-card use case from the operator: two accounts named for the two cards / cardholders. Each expense picks one.

### D3. Category model: flat list, operator-defined

Same shape as `expense_accounts` — name, slug, is_active, sort_order. Operator creates whatever taxonomy fits their org (Supplies, Software, Events, Insurance, etc.). No predefined defaults from Coterie; the wizard or admin UI can seed a starter list but doesn't have to.

No hierarchy in v1. If an org needs sub-categories, that's a later iteration with real demand.

### D4. Service shape mirrors MemberService

`ExpenseService::{create_expense, update_expense, delete_expense, list_expenses, get_expense}`. Each mutation:
- Validates at the service boundary (amount ≥ 0, ≤ MAX_EXPENSE_CENTS [pick a sensible cap], category exists + is active, account exists + is active, description non-empty).
- Writes via repo.
- Emits audit row with action `create_expense` / `update_expense` / `delete_expense`, entity_type `expense`, old_value/new_value carrying a summary (e.g., `"$30.00 / Supplies / Card 1 / Jane"`).
- No integration events (per the proposal — internal-only data).

`ExpenseCategoryService` and `ExpenseAccountService` are smaller — CRUD with audit, no list-time validation. Similar shape to `basic_type_service` after `a32` lands its audit emission.

### D5. Three reports, three SQL queries

**Monthly report** (`GET /portal/admin/finance/reports/monthly?year=YYYY&month=MM`):

- Per-account expense total: `SELECT account_id, SUM(amount_cents) FROM expenses WHERE spent_at >= date AND spent_at < date+1mo GROUP BY account_id`.
- Per-category expense total: same shape, grouped by category.
- Monthly income (for context): `SELECT SUM(amount_cents) FROM payments WHERE status = 'Completed' AND paid_at >= date AND paid_at < date+1mo`.
- Net: income - expense total.

Display: a single page with the three sections + a net line. No charts in v1; tables are fine.

**Annual report** (`GET /portal/admin/finance/reports/annual?year=YYYY`):

- Same sections aggregated over 12 months.
- Per-category breakdown of expenses (one row per category, full year total).
- Income broken into dues vs. donations (using `payments.kind`).
- Net for the year.

**Tax-prep export** (`GET /portal/admin/finance/reports/tax-prep?year=YYYY` as CSV):

Single CSV with all transactions for the year. Rows from three sources, all in one stream:

```csv
date,type,amount,description,counterparty,category,account,reference
2024-03-15,payment,150.00,Annual dues,Jane Doe <jane@…>,membership,stripe,pi_3abc…
2024-04-02,donation,50.00,Spring campaign,Bob Smith <bob@…>,donation,stripe,pi_3xyz…
2024-04-03,expense,30.50,Office supplies,,supplies,Debit Card 1 – Jane,
2024-04-12,refund,-25.00,Refund of mistaken charge,Carol Lee <carol@…>,membership,stripe,re_3def…
```

Sorted by date ASC. `type` is one of `payment | donation | refund | expense`. Negative amounts for refunds (they're outflows from the org's perspective). The accountant gets one file, can import to whatever they use.

### D6. Routing under /portal/admin/finance/

New top-level admin nav item "Finance" between "Billing" and "Audit Log" (or wherever fits the existing nav order). Under it:

- Overview / monthly report (default landing)
- Annual report
- Tax-prep export
- Expenses (list, new, edit, delete)
- Categories (CRUD)
- Accounts (CRUD)

All routes admin-only (existing `require_admin_redirect` middleware).

### D7. Currency handling

Inherit the existing payment-side convention: `currency: String` (3-letter ISO code) on expenses. Defaults to `USD`. No conversion logic; reports SHALL filter by a single currency in v1 (assume the org operates in one currency). If a future org needs multi-currency, that's a separate change.

### D8. Performance / scale

Even a high-activity org won't exceed ~thousands of expense rows per year. SQLite handles this trivially. Indexes on `spent_at` + `category_id` + `account_id` cover the common filters and reports. No need for caching, pre-aggregation, materialized views.

## Risks / Trade-offs

- **Risk**: an operator deletes a category that's referenced by existing expense rows. → Mitigation: the `expense_categories` delete handler refuses if rows reference it; offer a "deactivate" affordance instead (set `is_active = 0`). Same pattern as the existing `admin-types` capability already uses for membership types.
- **Risk**: tax-prep CSV grows unboundedly with multi-year data. → Acceptable for v1; one year per export is the default. If multi-year ever matters, paginate.
- **Risk**: combining payments + donations + refunds + expenses in one CSV exposes domain-modeling decisions to the accountant (e.g., is a "Pending" Stripe payment included? what about a refunded charge — does it appear as one row or two?). → Design v1 to be conservative: only `Completed` payments, only `Succeeded` refunds, only `Active` expenses. Document the inclusion criteria at the top of the CSV header line or in a sibling README link.
- **Risk**: the report's "net" line might disagree with the bank statement because of timing (cash basis vs. clearing date). → Document explicitly that Coterie reports cash basis on the spent_at / paid_at date — the accountant's responsibility to translate to whatever basis their books use.
- **Trade-off**: no receipt upload. Operators who want a paper trail need to keep receipts elsewhere (Google Drive, Dropbox folder per month, etc.). Reasonable v1 cost; revisit if operators ask.

## Migration Plan

Single PR.

1. New SQL migration for the three tables.
2. Domain types: `Expense`, `ExpenseCategory`, `ExpenseAccount` + their `Create*Request` / `Update*Request` shapes.
3. Three repositories.
4. Three services.
5. Admin handlers (list/new/edit/delete for expenses, CRUD for categories + accounts).
6. Three report endpoints (monthly, annual, tax-prep CSV).
7. Templates (Askama). Navigation update to add "Finance" item.
8. Integration tests: a fixed dataset (a few payments, donations, refunds, expenses) → expected monthly + annual totals + CSV row-by-row.
9. `cargo test`, `cargo clippy --deny warnings`, `cargo fmt --check`.
10. PR description includes an operator smoke walkthrough: create a category, create an account, record three expenses across a month, view the monthly report, download the tax-prep CSV for the year, eyeball that the totals match.
