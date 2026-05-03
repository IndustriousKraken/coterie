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

