## 1. Database migration

- [x] 1.1 Create `migrations/NNNN_expense_tracking.sql` with the three tables from `design.md` D1 (`expense_accounts`, `expense_categories`, `expenses`) plus indexes on `spent_at`, `category_id`, `account_id`.
- [x] 1.2 Smoke-test the migration against an in-memory SQLite.

## 2. Domain types

- [x] 2.1 New file `src/domain/expense.rs` containing `Expense`, `ExpenseCategory`, `ExpenseAccount` structs + their `Create*Request` / `Update*Request` shapes.
- [x] 2.2 Add the new types to `src/domain/mod.rs`'s public re-exports.

## 3. Repositories

- [x] 3.1 `src/repository/expense_repository.rs` — `trait ExpenseRepository` with: `create`, `update`, `delete`, `find_by_id`, `list` (with `ExpenseFilter { date_range, category_id, account_id, limit, offset }`), `sum_by_account(date_range)`, `sum_by_category(date_range)`.
- [x] 3.2 `src/repository/expense_category_repository.rs` — standard CRUD trait + `count_referencing_expenses(category_id)` for the soft-delete-check.
- [x] 3.3 `src/repository/expense_account_repository.rs` — same shape as the category repo.
- [x] 3.4 Add new trait declarations to `src/repository/mod.rs`. Document concurrency / idempotency on the traits per project conventions.
- [x] 3.5 Concrete implementations + repo tests (in-memory SQLite, following the established pattern from `tests/billing_dashboard_test.rs`).

## 4. Services

- [x] 4.1 `src/service/expense_service.rs` with `ExpenseService::{create_expense, update_expense, delete_expense, list_expenses, get_expense}`. Each mutation: validates inputs (amount ≥ 0, ≤ MAX_EXPENSE_CENTS [pick e.g. 10_000_000 = $100k], category + account both active, description non-empty); writes via repo; emits audit row.
- [x] 4.2 `src/service/expense_category_service.rs` with CRUD methods. Delete refuses if `count_referencing_expenses > 0`.
- [x] 4.3 `src/service/expense_account_service.rs` — same shape as category service.
- [x] 4.4 Wire all three services into `ServiceContext` and `AppState` (`FromRef` impls).

## 5. Admin handlers — Expenses CRUD

- [x] 5.1 `src/web/portal/admin/finance/expenses.rs`:
  - `GET /portal/admin/finance/expenses` — paginated list with filter form (date range, category, account dropdowns).
  - `GET /portal/admin/finance/expenses/new` — empty entry form.
  - `POST /portal/admin/finance/expenses` — submit; calls `create_expense`; redirects to the list.
  - `GET /portal/admin/finance/expenses/:id/edit` — pre-populated form.
  - `POST /portal/admin/finance/expenses/:id` — submit edit; calls `update_expense`.
  - `POST /portal/admin/finance/expenses/:id/delete` — calls `delete_expense`; redirects to list.

## 6. Admin handlers — Categories + Accounts CRUD

- [x] 6.1 `src/web/portal/admin/finance/categories.rs` — list + new + edit + delete + activate/deactivate routes.
- [x] 6.2 `src/web/portal/admin/finance/accounts.rs` — same shape.

## 7. Reports

- [x] 7.1 `src/web/portal/admin/finance/reports.rs`:
  - `GET /portal/admin/finance/reports/monthly?year=YYYY&month=MM` — renders per-account + per-category expense totals, monthly income from `payments`, net. Cash-basis caveat displayed.
  - `GET /portal/admin/finance/reports/annual?year=YYYY` — annual aggregation: per-category expense totals, income split by `payments.kind`, net.
  - `GET /portal/admin/finance/reports/tax-prep?year=YYYY` — CSV download per design D5. Builds the row stream by querying `payments` (Completed only), refunds (Successful only), and `expenses` for the year; merges sorted by date; serializes via `csv` crate.

## 8. Templates

- [x] 8.1 `templates/admin/finance/expenses_list.html` — paginated list with filter form.
- [x] 8.2 `templates/admin/finance/expense_form.html` — shared by new + edit.
- [x] 8.3 `templates/admin/finance/categories_list.html`, `categories_form.html`.
- [x] 8.4 `templates/admin/finance/accounts_list.html`, `accounts_form.html`.
- [x] 8.5 `templates/admin/finance/report_monthly.html`, `report_annual.html`.
- [x] 8.6 Navigation: add "Finance" item to the admin nav between "Billing" and "Audit Log" (or wherever fits the existing order).

## 9. Routing

- [x] 9.1 Register all new routes under `/portal/admin/finance/` in the portal admin router. They inherit the existing `require_admin_redirect` + CSRF middleware tree.
- [x] 9.2 Add a marker comment in the router noting that the rate-limiter is NOT applied here (these are not money-moving in the Stripe-charge sense — `money_limiter` belongs to endpoints that initiate charges).

## 10. Tests

- [x] 10.1 Repo tests: create/update/delete/list for each of the three repos.
- [x] 10.2 Service tests:
  - `create_expense_audits_and_inserts` — happy path.
  - `create_expense_rejects_negative_amount` — boundary.
  - `create_expense_rejects_inactive_category` — referential integrity.
  - `delete_category_with_existing_expenses_refuses` — soft-delete-check.
  - `update_expense_emits_audit_with_old_and_new` — audit shape.
- [x] 10.3 Report tests with a fixture dataset:
  - `monthly_report_sums_correctly` — fixed expenses + payments → expected totals per design D5 example.
  - `annual_report_aggregates_by_category` — multi-month expenses sum into single category row.
  - `tax_prep_csv_contains_expected_rows` — fixture year → expected CSV content per scenario in spec.
  - `tax_prep_csv_sorts_by_date` — assert sort order.
- [x] 10.4 Routing tests:
  - `non_admin_finance_routes_redirect_to_dashboard`.
  - `unauthenticated_finance_routes_redirect_to_login`.

## 11. Validation

- [x] 11.1 `cargo build --features test-utils` — clean.
- [x] 11.2 `cargo test --features test-utils` — all tests pass, including the new ones.
- [x] 11.3 `cargo clippy --features test-utils -- --deny warnings` — clean.
- [x] 11.4 `cargo fmt --check` — clean.
