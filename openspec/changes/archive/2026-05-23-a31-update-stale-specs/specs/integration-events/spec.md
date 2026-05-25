## RENAMED Requirements

- FROM: `### Requirement: Locus of integration-event dispatch varies by domain`
- TO: `### Requirement: Events for member operations are dispatched from MemberService`

## MODIFIED Requirements

### Requirement: Events for member operations are dispatched from MemberService

For member-mutation operations (`activate`, `suspend`, `update`, `expire_now`, `update_discord_id`, `resend_verification`, `create`, `bulk_import`, etc.), the **service** in `src/service/member_service.rs` SHALL call `self.integration_manager.handle_event(...)` after the repo update. The handler in `src/web/portal/admin/members/` SHALL NOT dispatch member-mutation events directly; the handler's only job is HTTP shape (extract inputs, call the service, render the response).

For payment operations, integration events (where applicable) SHALL be dispatched from `PaymentService` or `BillingService`. Payments do not produce `IntegrationEvent` variants directly today; admin alerts on billing failures are dispatched by `BillingService`.

This change aligns with the CLAUDE.md "side-effects in services" rule — both member operations and payments now follow it.

#### Scenario: New member-mutation method must dispatch events from the service

- **WHEN** a contributor adds a new member-mutation method to `MemberService`
- **THEN** the method MUST explicitly call `self.integration_manager.handle_event(...)` after the repo update; the handler does NOT (and SHALL NOT) dispatch events on its behalf

#### Scenario: Handler skips event dispatch by design

- **WHEN** a member-mutation handler is reviewed
- **THEN** the handler SHALL NOT contain any `integration_manager.handle_event` call for member events; that responsibility lives in the service

#### Scenario: BillingService dispatches AdminAlert on dunning

- **WHEN** the billing runner records the configured threshold of consecutive failures for a member
- **THEN** `BillingService` (not the handler) SHALL dispatch `IntegrationEvent::AdminAlert` so the admin-alert email integration sends a notification
