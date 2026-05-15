## MODIFIED Requirements

### Requirement: Locus of audit emission varies by domain

Audit-log emission SHALL live EITHER in the service layer OR in the handler, depending on the domain:

- **Payments**: emitted from `PaymentService` (e.g., `record_manual` calls `audit_service.log` internally). Adding a new payment-recording call site WITHOUT going through `PaymentService` would skip the audit.
- **Member operations** (activate, suspend, update, extend-dues, set-dues, expire-now, create, update-discord-id, resend-verification): emitted from `MemberService`. Adding a new member-mutation call site WITHOUT going through `MemberService` would skip the audit.
- **Settings, types, announcements, events**: emitted from the handler. The service-locus pattern has not yet been extended to these domains.
- **Logout**: emitted from the handler in `src/api/handlers/auth.rs`.

The architectural rule in `CLAUDE.md` is to emit audit rows from the service layer so a forgotten emission is structurally impossible. Payments and member operations follow this rule; settings, types, announcements, and events do not yet.

#### Scenario: New member-mutation call site routes through MemberService

- **WHEN** a contributor adds a new code path that mutates a member (admin action, scheduled job, etc.)
- **THEN** it SHALL call the appropriate `MemberService` method, which emits the audit row internally

#### Scenario: New payment recording site routes through PaymentService

- **WHEN** a contributor adds a new code path that records a payment
- **THEN** it SHALL call `PaymentService::record_manual` (or the equivalent service entry point), which emits the audit row internally

#### Scenario: New settings or types handler emits its own audit row

- **WHEN** a contributor adds a new settings, types, announcements, or events admin handler
- **THEN** the handler SHALL explicitly call `state.service_context.audit_service.log(...)` because no service wrapper does so on its behalf yet
