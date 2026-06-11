## MODIFIED Requirements

### Requirement: Locus of integration-event dispatch varies by domain

`IntegrationManager::handle_event` SHALL be called from EITHER the service layer OR the handler, depending on the domain:

- **Member operations**: dispatched from `MemberService`.
- **Event operations** (create with non-AdminOnly visibility): dispatched from `EventAdminService`. Adding a new event-mutation call site WITHOUT going through `EventAdminService` would skip the integration event.
- **Payment / billing operations**: dispatched from `BillingService`.
- **Announcement operations**: dispatched from the handler. The service-locus pattern has not yet been extended to this domain.
- **System notifications**: any subsystem MAY dispatch `IntegrationEvent::AdminAlert` directly.

#### Scenario: New event-mutation call site routes through EventAdminService

- **WHEN** a contributor adds a new code path that mutates an event
- **THEN** it SHALL call the appropriate `EventAdminService` method, which dispatches the integration event internally
