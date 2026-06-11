## ADDED Requirements

### Requirement: Configurable types — membership, event, announcement

The system SHALL provide three configurable type lists managed through the portal:

- Event types — `/portal/admin/types/event/...`
- Announcement types — `/portal/admin/types/announcement/...`
- Membership types — `/portal/admin/types/membership/...`

Each list SHALL support new, edit, and delete via the admin pages. The handler SHALL emit audit-log entries via `audit_service.log` after a successful service-layer mutation; the type services (`event_type_service`, `announcement_type_service`, `membership_type_service`) themselves do NOT emit audits.

#### Scenario: Creating a new event type is admin-only

- **WHEN** a non-admin requests `/portal/admin/types/event/new`
- **THEN** the request SHALL be redirected to `/portal/dashboard`

#### Scenario: Deleting a type that is referenced by existing rows is rejected or soft-deleted

- **WHEN** an admin attempts to delete a membership type that is currently assigned to members
- **THEN** the operation SHALL either reject with a clear error or perform a soft-delete that hides the type without invalidating existing references

### Requirement: Membership types govern dues amount and period

A membership type SHALL define the dues amount and dues period used by the recurring-billing flow. Changing a membership type's dues amount SHALL affect future charges; existing scheduled-payment rows SHALL retain the amount captured at scheduling time unless explicitly updated.

#### Scenario: Existing scheduled payments preserve their captured amount

- **WHEN** an admin changes the dues amount on a membership type
- **THEN** scheduled-payment rows that were created before the change SHALL keep their captured amount; the new amount SHALL apply to future scheduled rows
