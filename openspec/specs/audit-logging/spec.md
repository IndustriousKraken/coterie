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

- **Payments**: emitted from `PaymentService::record_manual` and from the per-event handlers inside `WebhookDispatcher`. Adding a new payment-recording call site WITHOUT going through one of these would skip the audit; the `payment-recording` capability spec lists the three permitted entry points.
- **Member operations** (activate, suspend, update, extend-dues, set-dues, expire-now, create, update-discord-id, resend-verification, bulk-import): emitted from `MemberService` in `src/service/member_service.rs`. The handler in `src/web/portal/admin/members/` SHALL NOT emit audit logs directly for these operations; the service handles it.
- **Settings, types, announcements, events**: emitted from the handler. (The `admin-types` capability's audit emission is a real bug today — the spec says the handler audits, but the code doesn't. A separate change adds the missing calls.)
- **Logout**: emitted from the handler in `src/api/handlers/auth.rs`.

This reflects current code structure: member operations and payments follow the CLAUDE.md "side-effects in services" rule; type/setting/announcement/event ops have audit in handlers.

#### Scenario: New member-mutation method must emit its own audit row from the service

- **WHEN** a contributor adds a new member-mutation method to `MemberService`
- **THEN** the method MUST explicitly call `self.audit_service.log(...)` after the repo update; no handler-side audit wrapper exists for member operations

#### Scenario: New payment recording site routes through one of the three entry points

- **WHEN** a contributor adds a new code path that records a payment
- **THEN** it SHALL call one of `PaymentService::record_manual`, `WebhookDispatcher::handle_*`, or `BillingService::process_scheduled_payment` — each emits the audit row internally; direct `payment_repo.create` calls are forbidden

#### Scenario: Handler does not emit duplicate audit for member operations

- **WHEN** an admin-member handler is reviewed
- **THEN** it SHALL NOT contain a direct `audit_service.log` call for member-mutation actions; the service emits the row

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

