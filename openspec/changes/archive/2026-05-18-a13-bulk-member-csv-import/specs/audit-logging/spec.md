## MODIFIED Requirements

### Requirement: Audit-log entry shape

Each `audit_logs` row SHALL include:

- `id` (UUID v4)
- `actor_id` (Option<UUID>) — the acting member, or NULL for system-initiated entries
- `action` (string) — e.g., `activate_member`, `suspend_member`, `update_member`, `record_payment`, `refund_payment`, `update_setting`, `logout`, `migrate_stripe_subs`, `export_members`, `import_member`, `import_members_batch`
- `entity_type` (string) — e.g., `member`, `payment`, `setting`, `session`
- `entity_id` (string) — the UUID or other identifier of the target. For aggregate batch actions (`export_members`, `import_members_batch`), the value `"*"` SHALL be used.
- `old_value` (Option<string>) — opaque before-state
- `new_value` (Option<string>) — opaque after-state, OR for aggregate actions a brief filter+count summary
- `ip_address` (Option<string>)
- `created_at` (timestamp, set by DB default)

#### Scenario: import_member row carries new-member email

- **WHEN** a row is successfully imported via bulk import
- **THEN** the inserted `audit_logs` row SHALL have `action = "import_member"`, `entity_type = "member"`, `entity_id = <new member uuid>`, and `new_value = Some(email)`

#### Scenario: import_members_batch row summarizes the batch

- **WHEN** a bulk import completes (regardless of partial failures)
- **THEN** the inserted aggregate row SHALL have `action = "import_members_batch"`, `entity_type = "member"`, `entity_id = "*"`, and `new_value` of the form `"file=<name>,succeeded=N,failed=M"`
