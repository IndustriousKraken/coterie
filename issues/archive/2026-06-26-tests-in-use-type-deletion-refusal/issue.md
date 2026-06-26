# tests-in-use-type-deletion-refusal

## Coverage gap

The configurable-type services refuse to delete a type that is still
referenced by existing rows, returning `AppError::Conflict`. The
**rejection branch is untested** — every existing delete test deletes a
type that is *not* in use, so only the happy "delete unused type" path
is exercised.

Three delete guards have no test for the in-use case:

- `MembershipTypeService::delete` —
  `src/service/membership_type_service.rs:100-117`. When
  `repo.count_usage(id) > 0` it returns
  `AppError::Conflict("Cannot delete membership type: {N} members still
  use this type. Deactivate instead.")`. Usage counts
  `members.membership_type_id = id`
  (`src/repository/membership_type_repository.rs:243`).
- `BasicTypeService::delete` for `BasicTypeKind::Event` —
  `src/service/basic_type_service.rs:67-78`, via
  `check_delete_unused_for_basic`
  (`src/service/configurable_types.rs:53-67`). When usage > 0 it returns
  `AppError::Conflict("Cannot delete event type: {N} events still use
  this type. Deactivate instead.")`. Usage counts
  `events.event_type_id = id`.
- `BasicTypeService::delete` for `BasicTypeKind::Announcement` — same
  function, message `"Cannot delete announcement type: {N} announcements
  still use this type. Deactivate instead."`. Usage counts
  `announcements.announcement_type_id = id`.

### Why this is the gap (not the happy path)

`tests/admin_types_audit_test.rs` already covers
`delete_event_type_writes_audit_row_with_old_name`,
`delete_announcement_type_writes_audit_row`, and
`delete_membership_type_writes_audit_row` — but each creates a type and
immediately deletes it with **no referencing rows**, so `count_usage`
returns 0 and the `Conflict` branch is never hit. (The expense-type
equivalents — `delete_category_with_existing_expenses_refuses` and
`delete_account_with_existing_expenses_refuses` in
`tests/expense_service_test.rs` — *are* covered; this gap is the
membership/event/announcement type guards only.)

## Acceptance criteria (against existing canon)

This pins behavior already required by the **admin-types** capability,
`openspec/specs/admin-types/spec.md`, Requirement **"Configurable
types — membership, event, announcement"**:

> #### Scenario: Deleting a type that is referenced by existing rows is rejected or soft-deleted
> - **WHEN** an admin attempts to delete a membership type that is currently assigned to members
> - **THEN** the operation SHALL either reject with a clear error or perform a soft-delete that hides the type without invalidating existing references

The implementation chooses the "reject with a clear error" arm
(`AppError::Conflict`). Acceptance: a referenced membership type, event
type, and announcement type each fail to delete with `AppError::Conflict`
whose message names the count and ends with `"Deactivate instead."`, and
the type row SHALL still exist after the rejected delete.

No production code changes. This is a test-only addition; no existing
test is modified or deleted.
