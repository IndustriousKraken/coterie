# admin-types Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Configurable types — membership, event, announcement

The system SHALL provide three configurable type lists managed through the portal:

- Event types — `/portal/admin/types/event/...`
- Announcement types — `/portal/admin/types/announcement/...`
- Membership types — `/portal/admin/types/membership/...`

Each list SHALL support new, edit, and delete via the admin pages. The handler SHALL emit audit-log entries via `audit_service.log` after a successful service-layer mutation; the type services themselves do NOT emit audits.

Audit row shape for each mutation:

| Operation | action string | entity_type | old_value | new_value |
|-----------|--------------|-------------|-----------|-----------|
| Create event type | `create_event_type` | `event_type` | None | type name |
| Update event type | `update_event_type` | `event_type` | old name | new name |
| Delete event type | `delete_event_type` | `event_type` | old name | None |
| Create announcement type | `create_announcement_type` | `announcement_type` | None | type name |
| Update announcement type | `update_announcement_type` | `announcement_type` | old name | new name |
| Delete announcement type | `delete_announcement_type` | `announcement_type` | old name | None |
| Create membership type | `create_membership_type` | `membership_type` | None | type name |
| Update membership type | `update_membership_type` | `membership_type` | old name | new name |
| Delete membership type | `delete_membership_type` | `membership_type` | old name | None |

For updates and deletes, the handler SHALL fetch the existing type's name BEFORE calling the service so `old_value` captures the pre-mutation state (otherwise the post-delete read would return nothing).

The event-type and announcement-type handler sets SHALL share a single implementation parameterized by `BasicTypeKind`. The membership-type handler set SHALL remain separate. URLs, form bodies, and rendered templates SHALL be unchanged from the pre-consolidation behavior.

#### Scenario: Creating a new event type is admin-only

- **WHEN** a non-admin requests `/portal/admin/types/event/new`
- **THEN** the request SHALL be redirected to `/portal/dashboard`

#### Scenario: Deleting a type that is referenced by existing rows is rejected or soft-deleted

- **WHEN** an admin attempts to delete a membership type that is currently assigned to members
- **THEN** the operation SHALL either reject with a clear error or perform a soft-delete that hides the type without invalidating existing references

#### Scenario: Event-type and announcement-type handlers share an implementation

- **WHEN** a contributor inspects the admin handlers for event types and announcement types
- **THEN** they SHALL find a single set of handler functions (or one handler per CRUD operation) parameterized by `BasicTypeKind`; the only divergence SHALL be the URL prefix, the kind threaded through the handler, and the template path

#### Scenario: Creating a type writes an audit row

- **WHEN** an admin creates a new membership type named "Annual" via the admin form
- **THEN** the `audit_logs` table SHALL gain one row with `actor_id = <admin's member id>`, `action = "create_membership_type"`, `entity_type = "membership_type"`, `entity_id = <new type's UUID>`, `old_value = NULL`, `new_value = "Annual"`

#### Scenario: Updating a type writes an audit row with the old and new names

- **WHEN** an admin renames an event type from "Workshop" to "Tournament"
- **THEN** the `audit_logs` table SHALL gain one row with `action = "update_event_type"`, `entity_type = "event_type"`, `old_value = "Workshop"`, `new_value = "Tournament"`

#### Scenario: Deleting a type writes an audit row with the old name

- **WHEN** an admin deletes an announcement type named "Newsletter"
- **THEN** the `audit_logs` table SHALL gain one row with `action = "delete_announcement_type"`, `entity_type = "announcement_type"`, `old_value = "Newsletter"`, `new_value = NULL`

#### Scenario: Audit insert failure does not roll back the mutation

- **WHEN** the audit-log insert errors (transient DB issue) after a successful type mutation
- **THEN** the type mutation SHALL remain committed; the audit failure SHALL be logged via `tracing` but not propagated to the client (per the `audit-logging` capability's fire-and-forget contract)

### Requirement: Membership types govern dues amount and period

A membership type SHALL define the dues amount and dues period used by the recurring-billing flow. Changing a membership type's dues amount SHALL affect future charges; existing scheduled-payment rows SHALL retain the amount captured at scheduling time unless explicitly updated.

#### Scenario: Existing scheduled payments preserve their captured amount

- **WHEN** an admin changes the dues amount on a membership type
- **THEN** scheduled-payment rows that were created before the change SHALL keep their captured amount; the new amount SHALL apply to future scheduled rows

### Requirement: Event-type and announcement-type behavior is implemented once

Event-type and announcement-type CRUD behavior — at the domain, repository, service, and admin-handler layers — SHALL share a single Rust-side implementation. The two kinds SHALL be discriminated by a `BasicTypeKind` enum (`Event`, `Announcement`) that selects the appropriate database table, usage-FK target, and human-readable display name.

A future change to color validation, unique-slug enforcement, sort-order policy, or delete-if-in-use behavior SHALL only need to land in one place; per-kind divergence SHALL NOT reappear without an explicit reason captured in the kind enum's data (e.g., a new `&'static str` accessor on `BasicTypeKind`).

#### Scenario: Adding a new validation rule lands once

- **WHEN** a contributor adds a validation rule to basic-type creation (e.g., reject names longer than 100 chars)
- **THEN** the rule SHALL be implemented in one place inside `BasicTypeService` and SHALL automatically apply to both event types and announcement types without per-kind code

#### Scenario: Membership-type service uses the shared color validator

- **WHEN** a membership-type create or update request is processed
- **THEN** the hex-color validation SHALL go through the same shared helper used by `BasicTypeService`; the error message format SHALL be consistent across all three configurable-type kinds

### Requirement: Repository and service consume the kind discriminator

The basic-type repository SHALL take `kind: BasicTypeKind` as a parameter on every method call so a single `SqliteBasicTypeRepository` instance can serve both kinds. The basic-type service SHALL bake the kind into each service instance so call-site ergonomics are unchanged: callers SHALL continue to invoke `state.service_context.event_type_service.create(...)` and `state.service_context.announcement_type_service.create(...)` exactly as today.

#### Scenario: Two service instances share one repository

- **WHEN** `ServiceContext::new` builds the configurable-type plumbing
- **THEN** it SHALL construct one `Arc<dyn BasicTypeRepository>` and two `BasicTypeService` instances (kind=Event and kind=Announcement) that share the same repo Arc

#### Scenario: Call sites do not move during the consolidation

- **WHEN** a handler calls `state.service_context.event_type_service.list(...)`
- **THEN** the call site SHALL continue to compile and produce identical output after the consolidation; the type discriminator SHALL be invisible to callers

### Requirement: Membership type stays a separate implementation

Membership types SHALL retain their own domain struct (`MembershipTypeConfig`), repository (`MembershipTypeRepository` + `SqliteMembershipTypeRepository`), and service (`MembershipTypeService`). Membership types carry billing-period and fee-cents fields, additional validation (billing-period parse, fee non-negative), and a different usage-FK target (`members.membership_type_id`); subsuming them into `BasicTypeService` would either constrain them or pollute the basic-type abstraction.

#### Scenario: Membership-type call sites are unaffected by the consolidation

- **WHEN** the public signup handler resolves a membership type via `state.service_context.membership_type_service.get_by_slug(...)`
- **THEN** the call site SHALL be unchanged after the consolidation; only the membership service's internal hex-color validator SHALL be replaced by a call to the shared helper

