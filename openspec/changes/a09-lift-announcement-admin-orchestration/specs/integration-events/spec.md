## MODIFIED Requirements

### Requirement: Locus of integration-event dispatch varies by domain

`IntegrationManager::handle_event` SHALL be called from EITHER the service layer OR the handler, depending on the domain:

- **Member operations**: dispatched from `MemberService`.
- **Event operations**: dispatched from `EventAdminService`.
- **Announcement operations** (publish, create-with-publish-now): dispatched from `AnnouncementAdminService`. Adding a new announcement-publish call site WITHOUT going through `AnnouncementAdminService` would skip the integration event.
- **Payment / billing operations**: dispatched from `BillingService`.
- **System notifications**: any subsystem MAY dispatch `IntegrationEvent::AdminAlert` directly.

#### Scenario: New announcement-publish call site routes through AnnouncementAdminService

- **WHEN** a contributor adds a new code path that publishes an announcement
- **THEN** it SHALL call `AnnouncementAdminService::publish` (or the publish-now create path), which dispatches the integration event internally
