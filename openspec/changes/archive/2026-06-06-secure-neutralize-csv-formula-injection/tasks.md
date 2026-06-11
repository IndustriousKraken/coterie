## 1. Add a formula-neutralizing CSV field writer

- [x] 1.1 In `src/web/portal/admin/csv.rs`, add `pub fn push_csv_user(out: &mut String, value: &str)`. It SHALL behave exactly like `push_csv` (always wrap in double quotes; double every internal `"`) but, when `value`'s FIRST character is one of `=`, `+`, `-`, `@`, `\t`, `\r`, or `\n`, it SHALL emit a single quote `'` immediately after the opening `"` and before the value, so a spreadsheet renders the cell as literal text. An empty value SHALL stay empty-quoted (`""`).
- [x] 1.2 Keep `push_csv` unchanged so non-user columns (numbers, dates, enums, booleans) are not altered.
- [x] 1.3 Add unit tests in `src/web/portal/admin/csv.rs` under `#[cfg(test)]`:
  - `push_csv_user_neutralizes_leading_equals`: input `=HYPERLINK("x","y")` produces a field beginning with `"'=HYPERLINK` (a `'` directly after the opening quote).
  - `push_csv_user_neutralizes_plus_minus_at_and_controls`: inputs `+1`, `-1`, `@x`, and a leading tab each get the `'` prefix.
  - `push_csv_user_leaves_ordinary_values_unquoted_inside`: input `O'Brien, Sean` produces `"O'Brien, Sean"` with NO leading `'` injected after the opening quote, and the internal apostrophe is unchanged.
  - `push_csv_user_doubles_internal_quotes`: input `say "hi"` produces `"say ""hi"""`.

## 2. Route member-export free-text columns through the new writer

- [x] 2.1 In `src/web/portal/admin/members/bulk.rs::build_members_csv`, replace `push_csv` with `push_csv_user` for the member-controlled free-text columns only: `r.email`, `r.username`, `r.full_name`, the `r.discord_id` value, and the `r.notes` value. Leave `id`, `status`, `membership_type`, the `joined_at` / `dues_paid_until` / `email_verified_at` timestamps, and the `is_admin` / `bypass_dues` booleans on plain `push_csv`.
- [x] 2.2 Update the import of `csv` helpers at the top of `build_members_csv` (currently `use crate::web::portal::admin::csv::push_csv;`) to also bring in `push_csv_user`.

## 3. Route audit-export free-text columns through the new writer

- [x] 3.1 In `src/web/portal/admin/audit.rs::audit_log_export`, replace `push_csv` with `push_csv_user` for the attacker-influenced free-text columns: `actor_name`, `entity_id`, `old_value`, and `new_value`. Leave the server-controlled columns (`created_at` timestamp, `actor_id`, `action`, `entity_type`, `ip_address`) on plain `push_csv`.
- [x] 3.2 Update the `use crate::web::portal::admin::csv::push_csv;` import in `src/web/portal/admin/audit.rs` to also import `push_csv_user`.

## 4. Regression test for the member export

- [x] 4.1 Add an integration test (e.g. `tests/admin_members_export_csv_injection.rs`, or extend the existing member-export test if one exists) that: creates a member whose `full_name` is `=cmd|'/c calc'!A1`, calls the export path so `build_members_csv` runs over rows including that member, and asserts the produced CSV contains the neutralized field `"'=cmd` (leading `'` after the opening quote) and does NOT contain a bare `,=cmd` / `"=cmd` start-of-field formula. Prefer asserting on `build_members_csv` directly with a constructed `MemberExportRow` if exercising the full HTTP path requires admin-session setup the test harness doesn't already provide.
