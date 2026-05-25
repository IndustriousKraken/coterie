# integration-events Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Events are typed enum variants in IntegrationEvent

`IntegrationEvent` SHALL be a Rust enum with the following variants:

- `MemberActivated(Member)`
- `MemberExpired(Member)`
- `MemberUpdated { old: Member, new: Member }`
- `EventPublished(Event)`
- `AnnouncementPublished(Announcement)`
- `AdminAlert { subject: String, body: String }`

Adding a new variant SHALL force every consumer match to be updated, preventing silently-dropped events.

#### Scenario: Adding a variant breaks consumer compilation

- **WHEN** a new variant is added to `IntegrationEvent`
- **THEN** every consumer match (Discord, UniFi, admin-alert email) without a default arm SHALL fail to compile

#### Scenario: AdminAlert is the free-form escape hatch

- **WHEN** any subsystem needs to surface an operational notification to admins without adding a dedicated variant
- **THEN** it SHALL dispatch `IntegrationEvent::AdminAlert { subject, body }`; this is the documented seam

### Requirement: IntegrationManager fans events out to registered integrations

`IntegrationManager::handle_event(event)` SHALL iterate every registered, enabled integration and call its `handle_event(&event)`. Integration failures SHALL be logged via `tracing::error!` and SHALL NOT halt processing of other integrations.

#### Scenario: One integration's failure does not block others

- **WHEN** Discord errors handling `MemberActivated` and UniFi is also registered
- **THEN** UniFi SHALL still receive the event; only the Discord failure SHALL be logged

#### Scenario: Disabled integration does not receive events

- **WHEN** an integration's `is_enabled()` returns `false` at registration time
- **THEN** it SHALL NOT be added to the manager's list; subsequent events SHALL skip it

### Requirement: Event consumers do not block the originating call

`handle_event` is `async` but called from handlers WITHOUT spawning. Consumers SHALL be implemented to be reasonably fast (millisecond-scale typical) so they do not noticeably extend handler latency. A consumer SHALL NOT roll back the originating action on failure; failures SHALL be logged and surfaced through admin-visible channels.

#### Scenario: Discord failure does not roll back activation

- **WHEN** an admin activates a member and the Discord integration's `handle_event` returns an error
- **THEN** the member SHALL remain Active; the failure SHALL be logged at error level and the integration SHALL recover via the next reconcile run

### Requirement: Events carry full domain values, not just ids

Variants like `MemberActivated(Member)` and `MemberUpdated { old, new }` SHALL carry full domain values so consumers do not need to re-query the database. `MemberUpdated` SHALL specifically carry both the pre-update and post-update snapshots so consumers can compute deltas (e.g., Discord role transitions).

#### Scenario: Discord role-change consumer reads old + new from event

- **WHEN** a `MemberUpdated { old, new }` event reaches the Discord integration
- **THEN** the integration SHALL compute role differences from the carried snapshots WITHOUT issuing additional DB reads

### Requirement: Locus of integration-event dispatch varies by domain

`IntegrationManager::handle_event` SHALL be called from EITHER the service layer OR the handler, depending on the domain:

- **Member operations**: dispatched from `MemberService`.
- **Event operations**: dispatched from `EventAdminService`.
- **Announcement operations**: dispatched from `AnnouncementAdminService`.
- **Payment / billing operations**:
  - Stripe-managed charge failures and subscription deletions: dispatched from `BillingService::Notifications` via `notify_subscription_payment_failed` and `notify_subscription_cancelled`.
  - Coterie-managed terminal charge failures: dispatched from `BillingService::AutoRenew::process_scheduled_payment`. (Per-retry transients are silent on the integration channel.)
  - Refunds: dispatched from `PaymentAdminService::refund`.
- **System notifications**: any subsystem MAY dispatch `IntegrationEvent::AdminAlert` directly.

#### Scenario: Coterie-managed terminal failure dispatches via AutoRenew

- **WHEN** a Coterie-managed scheduled payment hits the max-retries cap and transitions to `Failed`
- **THEN** the integration dispatch SHALL come from `AutoRenew::process_scheduled_payment`, not from a handler or another service

