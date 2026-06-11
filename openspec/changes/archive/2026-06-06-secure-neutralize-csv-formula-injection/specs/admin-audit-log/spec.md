## ADDED Requirements

### Requirement: Audit-log CSV export neutralizes spreadsheet formula injection

The audit-log CSV export (`GET /portal/admin/audit/export`) SHALL neutralize spreadsheet formula injection (CWE-1236) in its member-influenced free-text columns (`actor_name`, `entity_id`, `old_value`, `new_value`), whose values can be set by an attacker (for example, a member's chosen name recorded in an audit detail row).

Specifically, when one of these field values begins with a formula-trigger character (`=`, `+`, `-`, `@`) or with a control character (TAB, CR, LF), the export SHALL prefix the value with a single quote (`'`) inside the RFC 4180 double-quoting so spreadsheet applications render the cell as literal text. Server-controlled columns (`timestamp`, `actor_id`, `action`, `entity_type`, `ip_address`) SHALL NOT be altered by this neutralization.

#### Scenario: Formula-leading audit value is neutralized

- **WHEN** an `audit_logs` row's `new_value` begins with `=` and an admin exports the audit log
- **THEN** the exported CSV field SHALL begin with a single quote immediately after the opening double-quote so a spreadsheet opens it as text, not a formula
