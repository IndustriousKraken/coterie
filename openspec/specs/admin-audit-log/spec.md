# admin-audit-log Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Admin can view and export the audit log

The portal SHALL provide:
- `GET /portal/admin/audit` — paginated audit-log viewer with filters (actor, target type, date range).
- `GET /portal/admin/audit/export` — CSV export of the filtered set.

Both routes SHALL be admin-only via `require_admin_redirect`.

#### Scenario: Audit-log entry includes actor, target, and timestamp

- **WHEN** any admin action emits an audit-log entry from the service layer
- **THEN** the row SHALL record actor (member id), action type, target id, timestamp (UTC), and a structured details blob

#### Scenario: Export honors the active filter

- **WHEN** an admin exports the audit log with a date filter applied
- **THEN** the CSV SHALL include only rows in the filtered set

### Requirement: Audit log is append-only

The audit log SHALL be append-only at the data layer. Updates and deletes SHALL NOT be exposed via the portal or the repository trait.

#### Scenario: No update/delete API exists

- **WHEN** a contributor looks for an "update audit row" or "delete audit row" repository method
- **THEN** none SHALL exist; the trait SHALL expose only insert and read operations

### Requirement: Audit-log CSV export neutralizes spreadsheet formula injection

The audit-log CSV export (`GET /portal/admin/audit/export`) SHALL neutralize spreadsheet formula injection (CWE-1236) in its member-influenced free-text columns (`actor_name`, `entity_id`, `old_value`, `new_value`), whose values can be set by an attacker (for example, a member's chosen name recorded in an audit detail row).

Specifically, when one of these field values begins with a formula-trigger character (`=`, `+`, `-`, `@`) or with a control character (TAB, CR, LF), the export SHALL prefix the value with a single quote (`'`) inside the RFC 4180 double-quoting so spreadsheet applications render the cell as literal text. Server-controlled columns (`timestamp`, `actor_id`, `action`, `entity_type`, `ip_address`) SHALL NOT be altered by this neutralization.

#### Scenario: Formula-leading audit value is neutralized

- **WHEN** an `audit_logs` row's `new_value` begins with `=` and an admin exports the audit log
- **THEN** the exported CSV field SHALL begin with a single quote immediately after the opening double-quote so a spreadsheet opens it as text, not a formula

### Requirement: Audit-log rendering tolerates multi-byte free-text values

The audit-log viewer (`GET /portal/admin/audit`) and CSV export (`GET /portal/admin/audit/export`) SHALL render the member- and admin-influenced free-text columns (`old_value`, `new_value`) without panicking, regardless of the values' length or UTF-8 content. Any truncation applied for display SHALL cut on a UTF-8 character boundary, never on a raw byte index, so that a multi-byte character spanning the truncation point cannot cause a panic.

#### Scenario: Audit value with a multi-byte character at the truncation boundary renders safely

- **GIVEN** an `audit_logs` row whose `new_value` is longer than the display truncation limit and contains a multi-byte UTF-8 character (e.g. an emoji or accented letter) straddling the limit
- **WHEN** an admin loads `GET /portal/admin/audit` or `GET /portal/admin/audit/export`
- **THEN** the request SHALL complete without panicking and the displayed/exported detail SHALL be truncated on a character boundary (no partial multi-byte sequence)

#### Scenario: ASCII audit values are unaffected

- **WHEN** an `audit_logs` row's `new_value` is plain ASCII at or below the truncation limit
- **THEN** the rendered detail SHALL be byte-for-byte identical to the stored value

