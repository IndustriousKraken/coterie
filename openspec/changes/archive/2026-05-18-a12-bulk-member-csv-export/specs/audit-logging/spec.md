## MODIFIED Requirements

### Requirement: Audit-log entry shape

Each `audit_logs` row SHALL include:

- `id` (UUID v4)
- `actor_id` (Option<UUID>) — the acting member, or NULL for system-initiated entries
- `action` (string) — e.g., `activate_member`, `suspend_member`, `update_member`, `record_payment`, `refund_payment`, `update_setting`, `logout`, `migrate_stripe_subs`, `export_members`
- `entity_type` (string) — e.g., `member`, `payment`, `setting`, `session`
- `entity_id` (string) — the UUID or other identifier of the target. For aggregate actions (like `export_members`), the value `"*"` SHALL be used to indicate "applies to many rows."
- `old_value` (Option<string>) — opaque before-state
- `new_value` (Option<string>) — opaque after-state, OR for aggregate actions, a brief filter+count summary
- `ip_address` (Option<string>)
- `created_at` (timestamp, set by DB default)

#### Scenario: Export-members entry carries filter summary as new_value

- **WHEN** an admin exports the member roster with `?status=Active`
- **THEN** the inserted row SHALL have `action = "export_members"`, `entity_type = "member"`, `entity_id = "*"`, and `new_value` of the form `"status=Active,count=N"` where N is the exported row count
