## MODIFIED Requirements

### Requirement: Locus of integration-event dispatch varies by domain

`IntegrationManager::handle_event` SHALL be called from EITHER the service layer OR the handler, depending on the domain:

- **Member operations** (activate, suspend, update, extend-dues, set-dues, expire-now, update-discord-id): dispatched from `MemberService`. The service emits `MemberActivated` or `MemberUpdated { old, new }` as part of the operation's side-effect chain. Adding a new member-mutation call site WITHOUT going through `MemberService` would skip the integration event.
- **Payment / billing operations**: dispatched from `BillingService` (e.g., dunning failures fire `IntegrationEvent::AdminAlert`).
- **Event and announcement operations**: dispatched from the handler. The service-locus pattern has not yet been extended to these domains.
- **System notifications**: any subsystem MAY dispatch `IntegrationEvent::AdminAlert { subject, body }` directly via `integration_manager.handle_event`.

The architectural rule in `CLAUDE.md` is to dispatch from the service layer so a forgotten event is structurally impossible. Member operations and billing follow this rule; events and announcements do not yet.

#### Scenario: New member-mutation call site routes through MemberService

- **WHEN** a contributor adds a new code path that mutates a member
- **THEN** it SHALL call the appropriate `MemberService` method, which dispatches the integration event internally

#### Scenario: BillingService dispatches AdminAlert on dunning

- **WHEN** the billing runner records the configured threshold of consecutive failures for a member
- **THEN** `BillingService` (not the handler) SHALL dispatch `IntegrationEvent::AdminAlert` so the admin-alert email integration sends a notification

#### Scenario: New event or announcement handler dispatches explicitly

- **WHEN** a contributor adds a new event-mutation or announcement-mutation handler
- **THEN** the handler SHALL explicitly call `integration_manager.handle_event(...)` because no service wrapper does so on its behalf yet
