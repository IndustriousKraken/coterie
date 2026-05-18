## MODIFIED Requirements

### Requirement: Locus of audit emission varies by domain

Audit-log emission SHALL live EITHER in the service layer OR in the handler, depending on the domain:

- **Payments**: emitted from `PaymentService`.
- **Member operations**: emitted from `MemberService`.
- **Event operations** (create, update single, update series, delete, end series, delete series): emitted from `EventAdminService`. Adding a new event-mutation call site WITHOUT going through `EventAdminService` would skip the audit.
- **Settings, types, announcements**: emitted from the handler. The service-locus pattern has not yet been extended to these domains.
- **Logout**: emitted from the handler in `src/api/handlers/auth.rs`.

#### Scenario: New event-mutation call site routes through EventAdminService

- **WHEN** a contributor adds a new code path that mutates an event (admin action, scheduled job, etc.)
- **THEN** it SHALL call the appropriate `EventAdminService` method, which emits the audit row internally
