## ADDED Requirements

### Requirement: Audit-log rendering tolerates multi-byte free-text values

The audit-log viewer (`GET /portal/admin/audit`) and CSV export (`GET /portal/admin/audit/export`) SHALL render the member- and admin-influenced free-text columns (`old_value`, `new_value`) without panicking, regardless of the values' length or UTF-8 content. Any truncation applied for display SHALL cut on a UTF-8 character boundary, never on a raw byte index, so that a multi-byte character spanning the truncation point cannot cause a panic.

#### Scenario: Audit value with a multi-byte character at the truncation boundary renders safely

- **GIVEN** an `audit_logs` row whose `new_value` is longer than the display truncation limit and contains a multi-byte UTF-8 character (e.g. an emoji or accented letter) straddling the limit
- **WHEN** an admin loads `GET /portal/admin/audit` or `GET /portal/admin/audit/export`
- **THEN** the request SHALL complete without panicking and the displayed/exported detail SHALL be truncated on a character boundary (no partial multi-byte sequence)

#### Scenario: ASCII audit values are unaffected

- **WHEN** an `audit_logs` row's `new_value` is plain ASCII at or below the truncation limit
- **THEN** the rendered detail SHALL be byte-for-byte identical to the stored value
