## 1. Domain layer

- [ ] 1.1 In `src/domain/configurable_types.rs`, add `BasicType` struct with the fields shared by event and announcement types (id, name, slug, description, color, icon, sort_order, is_active, created_at, updated_at).
- [ ] 1.2 Add `BasicTypeKind` enum (`Event`, `Announcement`) with `Copy + Clone + Debug`, and const accessor methods returning `&'static str`: `table()`, `usage_table()`, `usage_fk()`, `display_name()`.
- [ ] 1.3 Add `CreateBasicTypeRequest` and `UpdateBasicTypeRequest` structs replacing the four parallel request types.
- [ ] 1.4 Add type aliases: `pub type EventTypeConfig = BasicType;`, `pub type AnnouncementTypeConfig = BasicType;`, `pub type CreateEventTypeRequest = CreateBasicTypeRequest;`, etc. for the four old request types.
- [ ] 1.5 Remove the now-redundant duplicate struct definitions for `EventTypeConfig`, `AnnouncementTypeConfig`, and the four request structs. The aliases replace them.

## 2. Shared validators

- [ ] 2.1 Create `src/service/configurable_types.rs` with `pub(crate)` helpers: `validate_hex_color_for_request(color: Option<&str>) -> Result<()>`, plus stubs for `check_unique_slug_for_basic` and `check_delete_unused_for_basic` (these are filled in once the basic-type repo trait exists in step 3).
- [ ] 2.2 Register `pub mod configurable_types;` in `src/service/mod.rs`.

## 3. Basic-type repository

- [ ] 3.1 Create `src/repository/basic_type_repository.rs` with `BasicTypeRepository` trait. Methods: `create(kind, request) -> Result<BasicType>`, `find_by_id(kind, id) -> Result<Option<BasicType>>`, `find_by_slug(kind, slug) -> Result<Option<BasicType>>`, `list(kind, include_inactive) -> Result<Vec<BasicType>>`, `update(kind, id, request) -> Result<BasicType>`, `delete(kind, id) -> Result<()>`, `count_usage(kind, id) -> Result<i64>`, `get_next_sort_order(kind) -> Result<i32>`.
- [ ] 3.2 Implement `SqliteBasicTypeRepository` with one Sqlite pool, building SQL strings via `format!` over `kind.table()` and `kind.usage_table()`. Verify the SQL shapes are byte-equivalent to the existing event-type and announcement-type repos.
- [ ] 3.3 Wire `pub mod basic_type_repository;` in `src/repository/mod.rs` and re-export.
- [ ] 3.4 Fill in `check_unique_slug_for_basic` and `check_delete_unused_for_basic` in `src/service/configurable_types.rs` now that the trait is defined.

## 4. Basic-type service

- [ ] 4.1 Create `src/service/basic_type_service.rs` with `BasicTypeService` holding `repo: Arc<dyn BasicTypeRepository>` and `kind: BasicTypeKind`. Constructor: `new(repo, kind)`.
- [ ] 4.2 Implement `list`, `get`, `get_by_slug`, `create`, `update`, `delete` methods that forward to the repo with the baked-in `kind` and use the shared validator helpers from step 2.
- [ ] 4.3 The `delete` method SHALL produce an error message using `kind.display_name()` so the user-visible string remains "Cannot delete event type: …" / "Cannot delete announcement type: …".
- [ ] 4.4 Register `pub mod basic_type_service;` in `src/service/mod.rs`.

## 5. ServiceContext / AppState plumbing

- [ ] 5.1 In `ServiceContext::new`, construct one `Arc<dyn BasicTypeRepository>` (the SqliteBasicTypeRepository) and two `Arc<BasicTypeService>` instances — one with `BasicTypeKind::Event`, one with `BasicTypeKind::Announcement`.
- [ ] 5.2 Replace the existing `event_type_service: Arc<EventTypeService>` and `announcement_type_service: Arc<AnnouncementTypeService>` fields with `event_type_service: Arc<BasicTypeService>` and `announcement_type_service: Arc<BasicTypeService>`. Field names stay the same so call sites don't move.
- [ ] 5.3 Verify `cargo build` compiles. Any remaining errors are call-site references to old method signatures or types — fix them.

## 6. Migrate MembershipTypeService to use shared color validator

- [ ] 6.1 Replace the inlined `validate_hex_color(...)` checks in `src/service/membership_type_service.rs` with calls to `validate_hex_color_for_request(...)` from the shared helpers module.
- [ ] 6.2 Confirm membership-type-specific validation (billing-period parse, fee non-negative, slug uniqueness against `membership_types`) stays inline in `MembershipTypeService` — those use the membership-specific repo, not the basic-type repo.

## 7. Delete the dead code

- [ ] 7.1 Delete `src/repository/event_type_repository.rs`.
- [ ] 7.2 Delete `src/repository/announcement_type_repository.rs`.
- [ ] 7.3 Delete `src/service/event_type_service.rs`.
- [ ] 7.4 Delete `src/service/announcement_type_service.rs`.
- [ ] 7.5 Remove the corresponding `pub mod` lines and re-exports in `src/repository/mod.rs` and `src/service/mod.rs`.
- [ ] 7.6 Run `cargo build` to confirm no stragglers.

## 8. Consolidate admin handlers

- [ ] 8.1 In `src/web/portal/admin/types.rs`, replace the parallel event-type and announcement-type handler sets (`admin_new_event_type_page`, `admin_create_event_type`, etc., and their announcement-type counterparts) with a single set of basic-type handlers parameterized by `BasicTypeKind`. Routes can either use a `:kind` path param or stay as two route registrations pointing at the same handler with kind hardcoded per route.
- [ ] 8.2 Adjust the route registration in `src/web/portal/mod.rs` so the same handler functions are reused for both `/types/event/...` and `/types/announcement/...`.
- [ ] 8.3 Verify membership-type handlers (`admin_new_membership_type_page`, `admin_create_membership_type`, etc.) are unchanged.
- [ ] 8.4 Confirm `fetch_event_types(state, ...)` and `fetch_announcement_types(state, ...)` (helpers in `types.rs` for the overview page) still produce the right `TypeInfo` lists; collapse them into a single `fetch_basic_types(state, kind, include_inactive)` if natural.

## 9. Test and verify

- [ ] 9.1 Run `cargo test --features test-utils`. Existing event-type and announcement-type tests SHALL pass without modification.
- [ ] 9.2 Add a new test asserting that the `delete` error message for a basic type with usage references contains the right display name (`event type` for kind=Event, `announcement type` for kind=Announcement).
- [ ] 9.3 Add a new test asserting that `list(kind=Event)` and `list(kind=Announcement)` query different tables and produce disjoint result sets.
- [ ] 9.4 Eyeball the final `src/web/portal/admin/types.rs` line count — expected target 350–400 lines (down from 587).
- [ ] 9.5 Eyeball the total lines removed from `src/repository/`, `src/service/`, and `src/domain/configurable_types.rs` — expected net reduction ≥ 400 lines.

## 10. Spec sync

- [ ] 10.1 Confirm the change's delta specs (`openspec/changes/consolidate-configurable-types/specs/admin-types/spec.md` and `domain-types/spec.md`) match the implemented behavior.
- [ ] 10.2 At archive time (`opsx:archive`), the new requirements about `BasicType`, `BasicTypeKind`, the unified service, and the parameterized handlers merge into `openspec/specs/admin-types/spec.md` and `openspec/specs/domain-types/spec.md`.
