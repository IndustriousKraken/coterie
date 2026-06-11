## Why

Coterie tracks one half of the books — income, via payments and donations. The other half — expenses — currently lives in spreadsheets, and reconciliation is a monthly chore that gets worse at tax time. Concrete pain from a live org running Coterie:

- Two debit cards in use by different people. Spreadsheets have to track who paid for what on which card. Monthly close means hand-correlating bank statements to the spreadsheet.
- Tax season requires assembling a combined view: Stripe receipts + refunds (already in Coterie) + tracked expenses (currently spreadsheets) → a single picture that helps determine tax-prep needs.
- No place in Coterie today to record "Jane bought $30 of supplies on Card 1 on April 3."

Adding expense tracking gives Coterie the income-plus-expenses view in one place, with the same audit + access controls already in use for payment data. The scope is narrow on purpose: this is a ledger, not an accounting suite. Coterie SHALL NOT apply IRS or state tax rules; it provides the data, the operator (or their accountant) draws conclusions.

## What Changes

- **New domain types**:
  - `Expense` — id, date, amount_cents, currency, description, category_id, account_id, notes, created_by, created_at, updated_at.
  - `ExpenseCategory` — id, name, slug, sort_order, is_active. Flat list (no hierarchy in v1).
  - `ExpenseAccount` — id, name (e.g., "Debit Card 1 – Jane"), is_active. Represents the funding source / instrument used to pay.
- **New repositories**: `ExpenseRepository`, `ExpenseCategoryRepository`, `ExpenseAccountRepository`. Standard CRUD + list-by-filter (date range, category, account).
- **New service**: `ExpenseService` with create/update/delete/list/filter. Audit emission via the established service-layer pattern. No integration events (expenses are operator-internal, not relevant to Discord/UniFi/etc.).
- **New admin pages** under `/portal/admin/finance/`:
  - `/expenses` — list with filters (date range, category, account); paginated.
  - `/expenses/new` — entry form.
  - `/expenses/:id/edit` — edit form.
  - `/expenses/:id/delete` — POST-only with audit.
  - `/expenses/categories` and `/expenses/accounts` — CRUD pages for the two lookup tables (similar shape to existing `/portal/admin/types/`).
- **Reconciliation reports** under `/portal/admin/finance/reports/`:
  - `/monthly?year=YYYY&month=MM` — per-account totals (debits + credits if you treat refunds as credits), expense totals by category, net for the month.
  - `/annual?year=YYYY` — same shape over the full year, with revenue (from `payments` + `donations`) on the income side and expenses on the outflow side. Cash-basis (date-of-payment, not date-of-event).
  - `/tax-prep?year=YYYY` — CSV download containing every transaction (payments, donations, refunds, expenses) for the year, with columns suitable for handing to an accountant: date, type, amount, description, member/payee, category, account, reference (Stripe charge ID for Stripe rows).
- **No UI for receipt upload** in v1. The TODO mentioned it; the operator's actual immediate need is data + reconciliation, not document storage. Receipts can be added in a follow-up if asked.

## Capabilities

### New Capabilities
- `expense-tracking`: record expenses with category + account attribution, filter and list them, and generate monthly / annual / tax-prep reconciliation reports that combine expense data with existing payment + donation income.

### Modified Capabilities
None. Expense tracking adds new surfaces; it doesn't change existing payment, donation, or admin behavior.

## Impact

- **Code**:
  - 3 new domain types + 3 new repositories + 1 new service + ~7 new admin handler routes + report queries (cross-cutting over `expenses`, `payments`, `donations`).
  - 1 SQL migration creating `expenses`, `expense_categories`, `expense_accounts` tables.
  - New templates for entry/edit/list/reports. CSV export uses the existing CSV scaffolding from `a12-bulk-member-csv-export` patterns.
- **Wire shape**: all new routes under `/portal/admin/finance/`. No existing route changes.
- **Tests**: repo tests (in-memory SQLite), service tests (audit emission, validation), report tests (a fixed dataset → expected monthly/annual totals).
- **Risk**: low. Pure addition; no shared schema mutations or cross-domain coupling beyond the report queries (which read existing tables, don't write).
- **Dependency**: none. Independent of a35 and prior queued changes.
- **Out of scope for v1** (capture for follow-ups):
  - Receipt image/PDF upload + storage.
  - Public transparency dashboard (some orgs want one; needs separate access-control discussion).
  - Hierarchical categories.
  - Bank-statement / card-export CSV import (would shave more time off reconciliation; defer until v1 ships and the workflow is observed).
  - Tax-rule application (explicit non-goal per the operator's framing).
  - Multi-currency accounting (all amounts assumed to be a single currency per the existing `Payment::currency` convention; expenses inherit the same `currency: String` field).
