## MODIFIED Requirements

### Requirement: Locus of audit emission varies by domain

Audit-log emission SHALL live EITHER in the service layer OR in the handler, depending on the domain:

- **Payments**: emitted from `PaymentService`.
- **Member operations**: emitted from `MemberService`.
- **Event operations**: emitted from `EventAdminService`.
- **Announcement operations** (create, update, delete, publish, unpublish): emitted from `AnnouncementAdminService`. Adding a new announcement-mutation call site WITHOUT going through `AnnouncementAdminService` would skip the audit.
- **Settings, types**: emitted from the handler. The service-locus pattern has not yet been extended to these domains.
- **Logout**: emitted from the handler in `src/api/handlers/auth.rs`.

#### Scenario: New announcement-mutation call site routes through AnnouncementAdminService

- **WHEN** a contributor adds a new code path that mutates an announcement
- **THEN** it SHALL call the appropriate `AnnouncementAdminService` method, which emits the audit row internally
