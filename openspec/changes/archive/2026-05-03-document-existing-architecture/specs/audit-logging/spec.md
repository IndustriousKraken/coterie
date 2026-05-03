## ADDED Requirements

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
- `action` (string) — e.g., `activate_member`, `suspend_member`, `update_member`, `record_payment`, `refund_payment`, `update_setting`, `logout`, `migrate_stripe_subs`
- `entity_type` (string) — e.g., `member`, `payment`, `setting`, `session`
- `entity_id` (string) — the UUID or other identifier of the target
- `old_value` (Option<string>) — opaque before-state, often a name/email or JSON blob
- `new_value` (Option<string>)
- `ip_address` (Option<string>)
- `created_at` (timestamp, set by DB default)

#### Scenario: Activate-member entry carries member email as new_value

- **WHEN** an admin activates a member
- **THEN** the inserted row SHALL have `action = "activate_member"`, `entity_type = "member"`, `entity_id = <member uuid>`, and `new_value` holding the member's email for human readability

### Requirement: Locus of audit emission varies by domain

Audit-log emission SHALL live EITHER in the service layer OR in the handler, depending on the domain:

- **Payments**: emitted from `PaymentService` (e.g., `record_manual` calls `audit_service.log` internally). Adding a new payment-recording call site WITHOUT going through `PaymentService` would skip the audit.
- **Member operations** (activate, suspend, update, extend-dues, set-dues, expire-now, create, update-discord-id, resend-verification): emitted from the **handler** in `src/web/portal/admin/members.rs` AFTER calling `member_repo` directly. There is no `MemberService` wrapping these calls.
- **Settings, types, announcements, events**: emitted from the handler.
- **Logout**: emitted from the handler in `src/api/handlers/auth.rs`.

This is observed behavior. The CLAUDE.md "side-effects live in services" rule is aspirational; payments follow it, the rest do not.

#### Scenario: New member-mutation handler must emit its own audit row

- **WHEN** a contributor adds a new member-mutation route to `src/web/portal/admin/members.rs`
- **THEN** the handler MUST explicitly call `state.service_context.audit_service.log(...)` because no member-service wrapper does so on its behalf

#### Scenario: New payment recording site routes through PaymentService

- **WHEN** a contributor adds a new code path that records a payment
- **THEN** it SHALL call `PaymentService::record_manual` (or the equivalent service entry point), which emits the audit row internally

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
