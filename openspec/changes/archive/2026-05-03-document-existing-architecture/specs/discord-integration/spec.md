## ADDED Requirements

### Requirement: Outbound-only integration driven by integration events

The Discord integration SHALL react to integration events emitted by services and SHALL call the Discord API outbound-only. The integration SHALL NOT expose any inbound HTTP surface from Discord into the application.

#### Scenario: Member status change produces a role update

- **WHEN** a member's status transitions to Active
- **THEN** the integration SHALL receive the integration event and call Discord to assign the configured roles for an Active member

#### Scenario: Discord cannot push state into the app

- **WHEN** a contributor proposes adding a webhook from Discord to the app
- **THEN** the change SHALL be re-scoped (e.g., as a different capability with its own auth model) — Discord integration today is outbound-only

### Requirement: Reconcile job aligns roles with member state

The integration SHALL provide a reconcile operation (admin-triggered via `POST /portal/admin/settings/discord/reconcile`) that walks all linked members and ensures Discord roles match current member state. The operation SHALL be admin-gated and audit-logged.

#### Scenario: Reconcile resolves drift caused by missed events

- **WHEN** an admin triggers reconcile after Discord was offline for a period
- **THEN** the integration SHALL bring roles back in sync with the application's current member state

### Requirement: Failures are logged but do not block the originating action

If a Discord call fails, the originating member-state change SHALL NOT be rolled back. The failure SHALL be logged and surfaced to admins via the integration's failure surface.

#### Scenario: Failed role assignment does not roll back member activation

- **WHEN** an admin activates a member and the subsequent Discord call fails
- **THEN** the member SHALL remain Active and the failure SHALL be visible in logs/admin alerts so it can be retried via reconcile
