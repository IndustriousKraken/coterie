# audit-logging Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: AuditService is a thin INSERT wrapper, fire-and-forget

`AuditService::log` SHALL insert an `audit_logs` row and SHALL NOT propagate errors to the caller. A DB failure on audit logging SHALL be recorded via `tracing` and otherwise swallowed; the primary operation has already happened, and dropping an audit row is strictly better than reverting or 500-ing the user.

#### Scenario: Audit insert failure does not affect the response

- **WHEN** the underlying INSERT fails (transient DB error)
- **THEN** the call SHALL return without error and the failure SHALL be logged at error level via `tracing`

#### Scenario: Caller does not need to handle a Result

- **WHEN** application code calls `audit_service.log(...)`
- **THEN** the function returns `()` (not `Result`); callers cannot accidentally fail-on-error

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

### Requirement: Locus of audit emission varies by domain

Audit-log emission SHALL live EITHER in the service layer OR in the handler, depending on the domain:

- **Payments**: emitted from `PaymentService` (for manual payment recording / waiving) and `PaymentAdminService` (for admin refunds). All payment-mutation paths route through a payment-flavored service.
- **Member operations**: emitted from `MemberService`.
- **Event operations**: emitted from `EventAdminService`.
- **Announcement operations**: emitted from `AnnouncementAdminService`.
- **Settings, types**: emitted from the handler. The service-locus pattern has not yet been extended to these domains.
- **Logout**: emitted from the handler in `src/api/handlers/auth.rs`.

After this change, every admin-mutation domain except settings/types follows the service-locus rule.

#### Scenario: Refund routes through PaymentAdminService

- **WHEN** an admin refunds a payment via `/portal/admin/payments/:id/refund`
- **THEN** the `refund_payment` audit row SHALL be emitted by `PaymentAdminService::refund`, not by the handler

### Requirement: Audit log is append-only at the application layer

The audit-log repository / service surface SHALL expose only insert and read operations. Update and delete SHALL NOT exist as application-level operations. Direct SQL `DELETE`/`UPDATE` against `audit_logs` from migrations or maintenance is out of scope of this rule.

#### Scenario: No update/delete in service surface

- **WHEN** a contributor inspects `AuditService`
- **THEN** they SHALL find only insertion and listing/filtering methods; modifying or removing past entries from application code SHALL NOT be possible

### Requirement: Logout writes a session audit row

Logout (both `/auth/logout` and `/logout`) SHALL write an `audit_logs` row with `action = "logout"`, `entity_type = "session"`, `entity_id = <session uuid>`, and `actor_id = <member uuid>` so admins can trace session lifecycle.

#### Scenario: Logout audit row is written before cookie is cleared

- **WHEN** an authenticated user POSTs to `/auth/logout`
- **THEN** the handler SHALL invoke `audit_service.log` with the session id BEFORE invalidating the session and clearing the cookie

