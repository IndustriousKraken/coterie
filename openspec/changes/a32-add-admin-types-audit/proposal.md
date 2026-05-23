## Why

The `admin-types` spec says: "The handler SHALL emit audit-log entries via `audit_service.log` after a successful service-layer mutation." But no audit emission exists anywhere in the basic-type or membership-type pipeline — handlers, services, and repositories all lack any `audit_service.log` call. A grep confirms zero matches.

This means today, an admin can rename, recolor, or delete a membership type (which governs dues amount + billing period for every member assigned to it) and there's no audit trail. Same for event types and announcement types. The mutations work, but a forensic question like "who changed the 'Annual' dues to $5?" has no answer.

This is a real missing-behavior bug, not a spec drift. The spec was always correct; the implementation just never landed the audit calls.

## What Changes

- Six handlers gain audit emission, one call after each successful service-layer mutation:
  - `admin_create_basic_type` (events + announcements share this) → `audit_service.log(actor, "create_event_type" | "create_announcement_type", "event_type" | "announcement_type", id, None, Some(&name), ip)`
  - `admin_update_basic_type` → `update_event_type` / `update_announcement_type`
  - `admin_delete_basic_type` → `delete_event_type` / `delete_announcement_type`
  - `admin_create_membership_type` → `create_membership_type`
  - `admin_update_membership_type` → `update_membership_type`
  - `admin_delete_membership_type` → `delete_membership_type`
- Each handler gains a `State(audit_service): State<Arc<AuditService>>` extractor (and similar for `SessionInfo` to get `ip_address` if not already extracted). The currently-unused `Extension(_current_user): Extension<CurrentUser>` becomes used (`current_user.member.id` is the `actor_id`).
- For updates and deletes, the handler reads the existing type's name before the mutation so the audit row's `old_value` carries the pre-change name. (Otherwise after delete there's nothing to read.)

## Capabilities

### New Capabilities
None.

### Modified Capabilities
None — this brings code into alignment with the existing `admin-types` capability spec; no spec requirement text changes.

## Impact

- **Code**:
  - ~6 handler modifications in `src/web/portal/admin/types.rs` (each adds 1 State extractor, 1 fetch-before-mutate where needed, 1 `audit_service.log` call).
  - Possibly a small `audit_action_for_kind` helper to centralize the action-string mapping (`Event` → `"event_type"`, `Announcement` → `"announcement_type"`) so the basic-type handlers don't duplicate the mapping.
- **Wire shape**: no change — same routes, same handlers, just an additional DB write per mutation.
- **Tests**: add integration tests asserting that each of the six mutations writes an audit row with the expected action/entity_type/entity_id/old_value/new_value.
- **Risk**: low. Pure addition; the audit call is fire-and-forget per the audit-logging spec (failures log via `tracing` but don't propagate), so even a misbehaving audit insert can't break the mutation.
- **Dependency**: none. Independent of a31 (a31 just updates `audit-logging` spec text; a32 adds missing audit calls per the unchanged `admin-types` spec).
