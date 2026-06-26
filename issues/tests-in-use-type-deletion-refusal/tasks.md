# Tasks

Add three `#[tokio::test]` functions to
`tests/admin_types_audit_test.rs`, reusing the existing `build_harness()`
(which exposes `pool`, `event_svc`, `announcement_svc`, `membership_svc`,
and a seeded member via `common::make_member`) and the `membership_form` /
`basic_form` helpers. Seed the referencing row with raw SQL (the same
direct-SQL pattern `tests/expiration_test.rs` uses), then call the
service's `delete` and assert the `Conflict`.

## 1. Membership-type in-use deletion is rejected
- [ ] 1.1 `delete_membership_type_in_use_is_rejected` — with `build_harness()`,
  create a membership type via `h.membership_svc.create(...)`; point a member
  at it (`UPDATE members SET membership_type_id = ? WHERE id = ?` using the
  harness member id, or insert a fresh member row referencing the type);
  then assert `h.membership_svc.delete(type_id).await` returns
  `Err(AppError::Conflict(msg))` where `msg.contains("Cannot delete membership type")`
  and `msg.contains("Deactivate instead.")`, AND that
  `h.membership_svc.get(type_id).await` still returns `Some(_)` (the row was
  not deleted).

## 2. Event-type in-use deletion is rejected
- [ ] 2.1 `delete_event_type_in_use_is_rejected` — create an event type via
  `h.event_svc.create(basic_form("Workshop"))`; insert one `events` row whose
  `event_type_id` equals the new type id (raw SQL, minimal required columns);
  then assert `h.event_svc.delete(type_id).await` returns
  `Err(AppError::Conflict(msg))` where `msg.contains("Cannot delete event type")`
  and `msg.contains("Deactivate instead.")`, AND `h.event_svc.get(type_id).await`
  still returns `Some(_)`.

## 3. Announcement-type in-use deletion is rejected
- [ ] 3.1 `delete_announcement_type_in_use_is_rejected` — create an
  announcement type via `h.announcement_svc.create(basic_form("Newsletter"))`;
  insert one `announcements` row whose `announcement_type_id` equals the new
  type id (raw SQL, minimal required columns); then assert
  `h.announcement_svc.delete(type_id).await` returns
  `Err(AppError::Conflict(msg))` where
  `msg.contains("Cannot delete announcement type")` and
  `msg.contains("Deactivate instead.")`, AND
  `h.announcement_svc.get(type_id).await` still returns `Some(_)`.
