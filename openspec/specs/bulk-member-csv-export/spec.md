# bulk-member-csv-export Specification

## Purpose
TBD - created by archiving change a12-bulk-member-csv-export. Update Purpose after archive.
## Requirements
### Requirement: Admins can download the member roster as CSV

The system SHALL expose `GET /portal/admin/members/export` returning the member roster as `text/csv; charset=utf-8`. The response SHALL include `Content-Disposition: attachment; filename="members-export-YYYY-MM-DD.csv"` so browsers download rather than render. The endpoint SHALL be gated by `require_admin_redirect`.

The CSV SHALL contain a header row followed by one row per member matching the current filter. The columns SHALL be, in this order:

`id, email, username, full_name, status, membership_type, joined_at, dues_paid_until, is_admin, bypass_dues, discord_id, email_verified_at, notes`

The CSV SHALL NOT include any credential field: no password hash, no TOTP secret, no recovery codes, no Stripe customer/subscription IDs.

#### Scenario: Admin gets a downloadable file

- **WHEN** an admin requests `GET /portal/admin/members/export`
- **THEN** the response SHALL have status 200, `Content-Type: text/csv; charset=utf-8`, and `Content-Disposition: attachment; filename="members-export-YYYY-MM-DD.csv"` with the date of the request

#### Scenario: Non-admin cannot reach the export

- **WHEN** an authenticated non-admin requests `GET /portal/admin/members/export`
- **THEN** the request SHALL be redirected to `/portal/dashboard` by `require_admin_redirect` (same as every other admin route)

#### Scenario: CSV escapes special characters

- **WHEN** a member's `full_name` is `"O'Brien, Sean"` or `notes` contain a comma, quote, or newline
- **THEN** the CSV writer SHALL escape these per RFC 4180 (double-quote the field; double up any internal double-quotes)

### Requirement: Export respects the same filters as the admin page

The export endpoint SHALL accept the same query string parameters as `/portal/admin/members` (`q` for search, `status` for status filter, `type` for membership type slug). The export SHALL include exactly the members that the filtered view would show, with pagination removed (`limit = unbounded`).

#### Scenario: Status filter narrows the export

- **WHEN** an admin requests `/portal/admin/members/export?status=Active`
- **THEN** the CSV SHALL contain only members whose status is Active

#### Scenario: No filter exports everything

- **WHEN** an admin requests `/portal/admin/members/export` with no query string
- **THEN** the CSV SHALL contain every member of every status (Active, Pending, Expired, Suspended, Honorary)

### Requirement: Exports are audit-logged

Every successful export SHALL write an `audit_logs` row with `action = "export_members"`, `entity_type = "member"`, `entity_id = "*"`, `actor_id = <admin's member id>`, and `new_value` summarizing the filter and the row count (e.g., `"status=Active,count=42"`).

#### Scenario: Successful export writes an audit row

- **WHEN** an admin successfully exports the roster
- **THEN** an `audit_logs` row SHALL be inserted with the fields above

### Requirement: Exported CSV neutralizes spreadsheet formula injection

The member roster export SHALL neutralize spreadsheet formula injection (CWE-1236) in its member-controlled free-text columns (`email`, `username`, `full_name`, `discord_id`, `notes`), which originate from the unauthenticated `POST /public/signup` endpoint and other member-editable surfaces with no character restrictions.

Specifically, when one of these field values begins with a formula-trigger character (`=`, `+`, `-`, `@`) or with a control character a spreadsheet may treat as starting a formula (TAB, CR, LF), the CSV writer SHALL prefix the value with a single quote (`'`) inside the RFC 4180 double-quoting so spreadsheet applications render the cell as literal text rather than evaluating it.

Numeric, timestamp, enum, and boolean columns (e.g. `id`, `status`, `joined_at`, `dues_paid_until`, `is_admin`, `bypass_dues`, `email_verified_at`) are not member-controlled and SHALL NOT be altered by this neutralization.

#### Scenario: Formula-leading member name is neutralized

- **WHEN** a member's `full_name` is `=HYPERLINK("http://evil.example","click")` and an admin exports the roster
- **THEN** the corresponding CSV field SHALL begin with a single quote immediately after the opening double-quote (e.g. `"'=HYPERLINK(...)"`) so a spreadsheet opens it as text, not a formula

#### Scenario: Ordinary values are not prefixed

- **WHEN** a member's `full_name` is `O'Brien, Sean` (no leading formula trigger)
- **THEN** the CSV field SHALL be quoted per RFC 4180 with no single quote injected after the opening double-quote, and the internal apostrophe SHALL be preserved unchanged

