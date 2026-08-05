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
- `EventUpdated(Event)`
- `EventDeleted(Event)`
- `AnnouncementPublished(Announcement)`
- `AnnouncementUpdated(Announcement)`
- `AnnouncementDeleted(Announcement)`
- `AdminAlert { subject: String, body: String }`

Adding a new variant SHALL force every consumer match to be updated, preventing silently-dropped events.

`EventPublished` fires when an event is created and is not `AdminOnly`. It SHALL
NOT be read as "this event is now public": it does not fire when an existing
event's visibility changes, and it does fire for members-only events. A consumer
that needs to know whether something is publicly visible SHALL determine that
from the visibility carried on the event, and SHALL learn of later changes from
`EventUpdated`.

The content update variants SHALL carry the post-update state only, and SHALL NOT
carry a pre-update snapshot. `MemberUpdated { old, new }` carries both because a
consumer must compute a delta from it — Discord derives role transitions that way
— and no consumer of an event or announcement change needs one: what a consumer
does is make its own copy match current state, which the new value alone
describes. Carrying a prior snapshot no one reads would double the content in
flight, and that content is the thing this capability most needs to keep small.

A single update variant SHALL cover retraction, late publication, and ordinary
content edits alike. Adding `Unpublished`, `Republished`, and `Rescheduled`
variants would multiply the enum without telling a consumer anything the current
state does not.

#### Scenario: Adding a variant breaks consumer compilation

- **WHEN** a new variant is added to `IntegrationEvent`
- **THEN** every consumer match (Discord, UniFi, admin-alert email) without a default arm SHALL fail to compile

#### Scenario: AdminAlert is the free-form escape hatch

- **WHEN** any subsystem needs to surface an operational notification to admins without adding a dedicated variant
- **THEN** it SHALL dispatch `IntegrationEvent::AdminAlert { subject, body }`; this is the documented seam

#### Scenario: Becoming public arrives as an update, not a publish

- **WHEN** an existing members-only event is changed to `Public`
- **THEN** `EventUpdated` SHALL be dispatched and `EventPublished` SHALL NOT be

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

Event and announcement variants are the exception, and the rule for them is drawn
by visibility rather than by variant. Such an event SHALL carry no more about an
item than that item's own visibility already discloses: an item that is `Public`
may carry its full public projection, one that is `MembersOnly` SHALL carry only
what `/public/events` would return for it, and one that is `AdminOnly` SHALL carry
only the identity and visibility a consumer needs to drop it.

This SHALL be enforced at dispatch, not left to consumers. Sending private content
outward and relying on each recipient to decline to publish it is a rule that
holds only as long as every current and future consumer implements it correctly,
and the failure is silent when one does not. Reducing the payload at the source
makes the wrong outcome unreachable instead of merely discouraged.

A consumer that needs more than this SHALL read it from a surface it is
authorized against, rather than being handed it in the notification.

#### Scenario: Discord role-change consumer reads old + new from event

- **WHEN** a `MemberUpdated { old, new }` event reaches the Discord integration
- **THEN** the integration SHALL compute role differences from the carried snapshots WITHOUT issuing additional DB reads

#### Scenario: An event withdrawn to admin-only carries no content forward

- **WHEN** an event changes from `Public` to `AdminOnly`
- **THEN** the dispatched `EventUpdated` SHALL identify the event and its new
  visibility without carrying its title, description, location, or image

#### Scenario: A members-only item carries only what the public API would show

- **WHEN** an event whose visibility is `MembersOnly` is updated
- **THEN** the dispatched event SHALL carry no more than `/public/events` returns
  for that item — the sanitized title, and no description, location, or image

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

### Requirement: Changes to public content are dispatched

A mutation SHALL dispatch the corresponding `IntegrationEvent` whenever it can
change what `/public/events` or `/public/announcements` returns for an item. This
is stated as a rule about observable output rather than as a list of methods,
because a list acquires gaps as new mutation paths are added and a gap here is
silent.

This SHALL include, at minimum: updating an event or announcement, deleting one,
changing an item's visibility or publication state, and the per-occurrence
exceptions that add or remove a materialized occurrence from the public feed.

Dispatch SHALL come from the service layer, consistent with how member and
announcement events are already dispatched, and SHALL happen after the repository
write succeeds so no consumer is told about a change that did not persist.

A mutation SHALL NOT dispatch when it changes nothing a public consumer could
observe — an edit confined to fields the public projection omits, or a change to
an item that was `AdminOnly` before and after.

#### Scenario: Editing an event notifies consumers

- **WHEN** an admin updates a public event's title or start time
- **THEN** `EventUpdated` SHALL be dispatched after the write

#### Scenario: Deleting an event notifies consumers

- **WHEN** an admin deletes an event that was not `AdminOnly`
- **THEN** `EventDeleted` SHALL be dispatched

#### Scenario: Cancelling an occurrence notifies consumers

- **WHEN** an admin cancels a single occurrence of a public recurring series, so
  that occurrence stops appearing in the public feed
- **THEN** a dispatch SHALL occur conveying that the occurrence is no longer
  publicly present

#### Scenario: A change with no public effect dispatches nothing

- **WHEN** an event that is `AdminOnly` both before and after is edited
- **THEN** no event or announcement variant SHALL be dispatched

#### Scenario: A failed write dispatches nothing

- **WHEN** a mutation fails at the repository layer
- **THEN** no `IntegrationEvent` SHALL be dispatched for it

