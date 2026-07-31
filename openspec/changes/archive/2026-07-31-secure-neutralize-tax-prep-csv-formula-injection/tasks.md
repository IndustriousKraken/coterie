# Tasks

## 1. Neutralize the free-text columns in the tax-prep CSV

- [x] 1.1 In `src/web/portal/admin/finance/reports.rs`, change the import on
  line 32 from `portal::admin::{csv::push_csv, partials}` to bring in both
  helpers: `portal::admin::{csv::{push_csv, push_csv_user}, partials}`.
- [x] 1.2 In `tax_prep_csv`'s serialize loop, switch the four free-text
  columns to `push_csv_user`: `&r.description` (line 336), `&r.counterparty`
  (line 338), `&r.category` (line 340), and `&r.account` (line 342).
- [x] 1.3 Leave `&r.date...` (line 330), `r.type_str` (line 332), the
  formatted `amount` (line 334), and `&r.reference` (line 344) on plain
  `push_csv` — they are server-controlled and must not be altered.
- [x] 1.4 Add a short comment above the loop naming which columns are
  member-influenced and which are server-controlled, matching the comment
  already present at `src/web/portal/admin/audit.rs:205-209`.

## 2. Test the neutralization end to end

- [x] 2.1 Add an integration test `tax_prep_csv_neutralizes_formula_injection`
  (new file `tests/tax_prep_csv_injection_test.rs`, following the setup shape
  of `tests/admin_member_export_test.rs`): seed a completed public-donation
  payment whose `donor_name` is `=HYPERLINK("http://evil","x")`, request
  `GET /portal/admin/finance/reports/tax-prep?year=<seeded year>` as an admin,
  and assert the `counterparty` and `description` cells begin with `"'=` —
  i.e. a single quote directly after the opening double-quote.
- [x] 2.2 In the same test, seed an expense whose category name begins with
  `+` and assert the `category` cell is likewise neutralized.
- [x] 2.3 Assert in the same test that an ordinary row is unchanged: a
  `counterparty` of `O'Brien, Sean` renders as `"O'Brien, Sean"` (no injected
  `'`), and the `amount` cell for a `-25.00` refund is still `"-25.00"` with
  no leading `'` — the server-controlled columns must not be rewritten.
- [x] 2.4 Run `cargo test --test tax_prep_csv_injection_test` and confirm it
  passes; run `cargo test` to confirm no existing golden/CSV test regressed.
