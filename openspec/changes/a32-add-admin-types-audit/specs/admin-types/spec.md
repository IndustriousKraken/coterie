## MODIFIED Requirements

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
