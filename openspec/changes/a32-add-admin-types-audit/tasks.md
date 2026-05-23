## 1. Helper for kind-based audit strings

- [ ] 1.1 In `src/web/portal/admin/types.rs` (or a small adjacent helper), add a `fn audit_strings_for_kind(kind: BasicTypeKind, op: &'static str) -> (String, &'static str)` that returns `(action_string, entity_type)` per the table in `specs/admin-types/spec.md`. Skip if inlining reads cleaner.

## 2. Wire AuditService through handler signatures

- [ ] 2.1 Add `State(audit_service): State<Arc<AuditService>>` to each of the 6 mutation handlers.
- [ ] 2.2 Confirm `AppState` (or whichever state type Axum extracts from) implements `FromRef<...>` for `Arc<AuditService>`. If not, add the `FromRef` impl — but it almost certainly already exists since other handlers use audit.
- [ ] 2.3 For IP capture: add `Extension(session_info): Extension<SessionInfo>` (or whichever pattern the codebase uses for the request's IP — match what other audited handlers do).
- [ ] 2.4 Drop the leading underscore on `_current_user` so the variable is used.

## 3. admin_create_basic_type — add audit

- [ ] 3.1 After `Ok(_)` from `svc.create(request).await`, read the created type's name (the service returns it — check the return shape) and bind to a local.
- [ ] 3.2 Call `audit_service.log(Some(current_user.member.id), &action_string, entity_type, &id.to_string(), None, Some(&name), session_info.ip_address.as_deref()).await;` per the spec table.
- [ ] 3.3 Verify the create's return value includes the new UUID; if not, fetch it post-create.

## 4. admin_update_basic_type — add audit with old/new names

- [ ] 4.1 BEFORE calling `svc.update(...)`, call `svc.find_by_id(id)` (or equivalent) to fetch the existing type's name. Bind to `old_name`.
- [ ] 4.2 After successful update, the form's `name` field is the new name (or the service's return value if it gives back the updated row).
- [ ] 4.3 Call `audit_service.log(Some(current_user.member.id), "update_event_type" | "update_announcement_type", entity_type, &id.to_string(), Some(&old_name), Some(&new_name), session_info.ip_address.as_deref()).await;`.

## 5. admin_delete_basic_type — add audit with old name

- [ ] 5.1 BEFORE calling `svc.delete(id)`, fetch the type's name. Bind to `old_name`.
- [ ] 5.2 After successful delete: `audit_service.log(Some(current_user.member.id), "delete_event_type" | "delete_announcement_type", entity_type, &id.to_string(), Some(&old_name), None, session_info.ip_address.as_deref()).await;`.

## 6. admin_create_membership_type — add audit

- [ ] 6.1 Same pattern as section 3, but action = `"create_membership_type"`, entity_type = `"membership_type"`.

## 7. admin_update_membership_type — add audit

- [ ] 7.1 Same pattern as section 4, with membership-type action/entity strings.

## 8. admin_delete_membership_type — add audit

- [ ] 8.1 Same pattern as section 5, with membership-type action/entity strings.

## 9. Tests

- [ ] 9.1 Create `tests/admin_types_audit_test.rs` (or extend an existing test file).
- [ ] 9.2 Test: create event type → audit row exists with `action=create_event_type`, `entity_type=event_type`, `new_value=<name>`, `old_value=NULL`.
- [ ] 9.3 Test: update event type → audit row with `action=update_event_type`, `old_value=<old name>`, `new_value=<new name>`.
- [ ] 9.4 Test: delete event type → audit row with `action=delete_event_type`, `old_value=<name>`, `new_value=NULL`.
- [ ] 9.5 Tests for announcement type and membership type (one create, one update, one delete each — 6 more tests).
- [ ] 9.6 Test: when `audit_service.log` errors internally (use a mock or feature-flagged failing audit service), the type mutation still completes successfully.

## 10. Validation

- [ ] 10.1 `cargo build --features test-utils` — clean.
- [ ] 10.2 `cargo test --features test-utils` — all tests pass, including the new ones.
- [ ] 10.3 `cargo clippy --features test-utils -- --deny warnings` — clean.
- [ ] 10.4 `cargo fmt --check` — clean.
- [ ] 10.5 Grep verification: `grep -n "audit_service" src/web/portal/admin/types.rs` returns at least one match per mutation handler.
