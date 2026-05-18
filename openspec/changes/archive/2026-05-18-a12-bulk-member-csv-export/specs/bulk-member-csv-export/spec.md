## ADDED Requirements

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
