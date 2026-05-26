## ADDED Requirements

### Requirement: Expenses are first-class records with category and account attribution

The system SHALL provide an `expenses` table and an `Expense` domain type with fields: `id` (UUID), `spent_at` (timestamp of the expense), `amount_cents` (non-negative integer), `currency` (3-letter ISO code, default `USD`), `description` (required, non-empty), `category_id` (FK to `expense_categories`), `account_id` (FK to `expense_accounts`), `notes` (optional free text), `created_by` (FK to `members`), `created_at`, `updated_at`.

`amount_cents` SHALL be ≥ 0. Refunds and other outflows that reverse income SHALL go through the existing payments/refund flow, not through `expenses`.

#### Scenario: Recording an expense persists every field

- **WHEN** an admin submits the expense entry form with date, amount, description, category, account, and optional notes
- **THEN** an `expenses` row SHALL be inserted with all six fields populated and `created_by = <admin's id>`, `created_at = now()`

#### Scenario: Negative amount is rejected

- **WHEN** an admin submits an expense form with `amount_cents = -100`
- **THEN** the service SHALL reject the submission with a clear error; no row is inserted

### Requirement: ExpenseCategory and ExpenseAccount are operator-managed lookup tables

The system SHALL provide `expense_categories` and `expense_accounts` tables, each with `id`, `name` (unique), `is_active`, `sort_order`, `created_at`. `expense_categories` additionally has a `slug` (unique).

Categories and accounts SHALL be CRUD-managed via admin pages similar in shape to the existing `admin-types` capability (basic types).

Soft-delete semantics: when an admin "deletes" a category or account that is referenced by existing expense rows, the operation SHALL refuse and offer to deactivate (`is_active = 0`) instead. Inactive categories/accounts continue to display on existing expense rows but are hidden from the new-expense form's dropdown.

#### Scenario: Deleting a referenced category is refused

- **WHEN** an admin attempts to delete an `expense_categories` row that has ≥1 `expenses` row referencing it
- **THEN** the delete SHALL fail with a clear error; the UI offers a "deactivate" affordance instead

#### Scenario: Inactive category hidden from new-expense form

- **WHEN** an admin loads the new-expense form
- **THEN** the category dropdown SHALL only include rows with `is_active = 1`; previously-recorded expenses referencing an inactive category continue to render its name on the list/edit pages

### Requirement: ExpenseService emits audit on every mutation

Every expense mutation (create, update, delete) SHALL emit an audit row via `audit_service.log` per the `audit-logging` capability. Action strings: `create_expense`, `update_expense`, `delete_expense`. Entity type: `expense`. Old/new values carry a human-readable summary (e.g., `"$30.00 / Supplies / Card 1 / 2024-04-03"`).

Category and account mutations similarly emit `create_expense_category` / `update_expense_category` / `delete_expense_category` and `create_expense_account` / `update_expense_account` / `delete_expense_account`.

#### Scenario: Recording an expense writes an audit row

- **WHEN** an admin records a new expense for $30.00 under category Supplies on account Card 1
- **THEN** the `audit_logs` table SHALL gain one row with `action = "create_expense"`, `entity_type = "expense"`, `entity_id = <new expense uuid>`, `new_value` containing the summary string

### Requirement: Monthly reconciliation report

`GET /portal/admin/finance/reports/monthly?year=YYYY&month=MM` SHALL render a single-page report containing:

- Per-account expense totals for the requested month (one row per account that had at least one expense).
- Per-category expense totals for the requested month.
- Total monthly income from `payments` where `status = 'Completed'` and `paid_at` falls within the month.
- Net (income - expense total).

The report SHALL be cash-basis: expenses count on their `spent_at` date; income counts on `payments.paid_at`. This SHALL be stated on the page so an operator comparing against a bank statement understands the basis.

#### Scenario: Monthly report sums correctly

- **GIVEN** a fixture month with three expenses ($30 + $50 + $20) all on the same account, two on category Supplies and one on category Software, and one $200 completed payment
- **WHEN** the monthly report is rendered for that month
- **THEN** the account row SHALL show $100.00 total; Supplies row $80.00; Software row $20.00; income $200.00; net $100.00

### Requirement: Annual reconciliation report

`GET /portal/admin/finance/reports/annual?year=YYYY` SHALL render a single-page report containing:

- Per-category expense totals across the full year.
- Income broken into dues vs. donations (via `payments.kind`).
- Net for the year.

Same cash-basis convention as the monthly report.

#### Scenario: Annual report aggregates by category across months

- **GIVEN** expenses in category Supplies across multiple months totaling $500 for the year
- **WHEN** the annual report is rendered
- **THEN** the Supplies row SHALL show $500.00 regardless of how the expenses were distributed across months

### Requirement: Tax-prep CSV export combines income, refunds, and expenses

`GET /portal/admin/finance/reports/tax-prep?year=YYYY` SHALL produce a CSV download. The CSV SHALL contain one row per transaction across `payments`, `donations`, refunds, and `expenses` for the requested year, sorted by transaction date ASC.

Columns: `date,type,amount,description,counterparty,category,account,reference`.

- `type` is one of `payment`, `donation`, `refund`, `expense`.
- `amount` is dollars with two decimal places. Refunds use NEGATIVE amounts (they are outflows from the org's perspective).
- `reference` carries the Stripe charge/refund/payment-intent ID for Stripe-sourced rows; empty for `expense` rows.

Inclusion criteria (documented in the response or a sibling note):
- Only `payments.status = 'Completed'`.
- Only successful refunds.
- All `expenses` rows.

#### Scenario: CSV contains expected row for each transaction type

- **GIVEN** a year with one $150 completed payment, one $50 donation, one $25 refund, and one $30 expense
- **WHEN** the tax-prep CSV is generated for that year
- **THEN** the CSV SHALL contain exactly 4 data rows (plus header); one with `type=payment, amount=150.00`; one with `type=donation, amount=50.00`; one with `type=refund, amount=-25.00`; one with `type=expense, amount=30.00`

#### Scenario: Rows are sorted by date ascending

- **WHEN** the tax-prep CSV is generated for a year with transactions on various dates
- **THEN** the rows SHALL be sorted by the `date` column in ascending order

### Requirement: Finance admin routes require admin authentication

All routes under `/portal/admin/finance/` SHALL require admin authentication and inherit the portal's existing CSRF + admin-redirect middleware. Non-admin members redirected to `/portal/dashboard`; unauthenticated requests redirected to `/login`.

#### Scenario: Non-admin member cannot access finance pages

- **WHEN** an authenticated non-admin member requests `/portal/admin/finance/expenses`
- **THEN** the request SHALL be redirected to `/portal/dashboard`

#### Scenario: Unauthenticated request redirected to login

- **WHEN** an unauthenticated request hits any `/portal/admin/finance/*` route
- **THEN** the request SHALL be redirected to `/login`
