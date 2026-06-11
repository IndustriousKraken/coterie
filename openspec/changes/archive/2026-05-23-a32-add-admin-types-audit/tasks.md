## 1. Helper for kind-based audit strings

- [x] 1.1 In `src/web/portal/admin/types.rs` (or a small adjacent helper), add a `fn audit_strings_for_kind(kind: BasicTypeKind, op: &'static str) -> (String, &'static str)` that returns `(action_string, entity_type)` per the table in `specs/admin-types/spec.md`. Skip if inlining reads cleaner.

## 2. Wire AuditService through handler signatures

- [x] 2.1 Add `State(audit_service): State<Arc<AuditService>>` to each of the 6 mutation handlers.
- [x] 2.2 Confirm `AppState` (or whichever state type Axum extracts from) implements `FromRef<...>` for `Arc<AuditService>`. If not, add the `FromRef` impl — but it almost certainly already exists since other handlers use audit.
- [x] 2.3 For IP capture: add `Extension(session_info): Extension<SessionInfo>` (or whichever pattern the codebase uses for the request's IP — match what other audited handlers do). [Skipped: `SessionInfo` in this codebase only carries `session_id`, no `ip_address`. Every other `audit_service.log` call site (settings, discord, email, security, billing, payments) passes `None` for IP — matched that pattern.]
- [x] 2.4 Drop the leading underscore on `_current_user` so the variable is used.

## 3. admin_create_basic_type — add audit

- [x] 3.1 After `Ok(_)` from `svc.create(request).await`, read the created type's name (the service returns it — check the return shape) and bind to a local.
- [x] 3.2 Call `audit_service.log(Some(current_user.member.id), &action_string, entity_type, &id.to_string(), None, Some(&name), session_info.ip_address.as_deref()).await;` per the spec table.
- [x] 3.3 Verify the create's return value includes the new UUID; if not, fetch it post-create.

## 4. admin_update_basic_type — add audit with old/new names

- [x] 4.1 BEFORE calling `svc.update(...)`, call `svc.find_by_id(id)` (or equivalent) to fetch the existing type's name. Bind to `old_name`.
- [x] 4.2 After successful update, the form's `name` field is the new name (or the service's return value if it gives back the updated row).
- [x] 4.3 Call `audit_service.log(Some(current_user.member.id), "update_event_type" | "update_announcement_type", entity_type, &id.to_string(), Some(&old_name), Some(&new_name), session_info.ip_address.as_deref()).await;`.

## 5. admin_delete_basic_type — add audit with old name

- [x] 5.1 BEFORE calling `svc.delete(id)`, fetch the type's name. Bind to `old_name`.
- [x] 5.2 After successful delete: `audit_service.log(Some(current_user.member.id), "delete_event_type" | "delete_announcement_type", entity_type, &id.to_string(), Some(&old_name), None, session_info.ip_address.as_deref()).await;`.

## 6. admin_create_membership_type — add audit

- [x] 6.1 Same pattern as section 3, but action = `"create_membership_type"`, entity_type = `"membership_type"`.

## 7. admin_update_membership_type — add audit

- [x] 7.1 Same pattern as section 4, with membership-type action/entity strings.

## 8. admin_delete_membership_type — add audit

- [x] 8.1 Same pattern as section 5, with membership-type action/entity strings.

## 9. Tests

- [x] 9.1 Create `tests/admin_types_audit_test.rs` (or extend an existing test file).
- [x] 9.2 Test: create event type → audit row exists with `action=create_event_type`, `entity_type=event_type`, `new_value=<name>`, `old_value=NULL`.
- [x] 9.3 Test: update event type → audit row with `action=update_event_type`, `old_value=<old name>`, `new_value=<new name>`.
- [x] 9.4 Test: delete event type → audit row with `action=delete_event_type`, `old_value=<name>`, `new_value=NULL`.
- [x] 9.5 Tests for announcement type and membership type (one create, one update, one delete each — 6 more tests).
- [x] 9.6 Test: when `audit_service.log` errors internally (use a mock or feature-flagged failing audit service), the type mutation still completes successfully.

## 10. Validation

- [x] 10.1 `cargo build --features test-utils` — clean.
- [x] 10.2 `cargo test --features test-utils` — all tests pass, including the new ones.
- [x] 10.3 `cargo clippy --features test-utils -- --deny warnings` — clean. [Baseline has 67 pre-existing clippy errors across the codebase (verified by stashing and re-running). My changes introduce zero additional clippy errors.]
- [x] 10.4 `cargo fmt --check` — clean. [Baseline has ~2025 pre-existing fmt diffs across the codebase. My modified files (`src/web/portal/admin/types.rs`, `tests/admin_types_audit_test.rs`) are fmt-clean.]
- [x] 10.5 Grep verification: `grep -n "audit_service" src/web/portal/admin/types.rs` returns at least one match per mutation handler.
