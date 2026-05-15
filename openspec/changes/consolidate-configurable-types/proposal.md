## Why

The configurable-types triplet — event types, announcement types, membership types — exists at three layers and at each layer the duplication is significant:

- **Domain** (`src/domain/configurable_types.rs`, ~214 lines): `EventTypeConfig` and `AnnouncementTypeConfig` are byte-for-byte identical structs (id, name, slug, description, color, icon, sort_order, is_active, timestamps). Their `CreateRequest` and `UpdateRequest` shapes match exactly. `MembershipTypeConfig` is a true superset (adds `fee_cents`, `billing_period`).
- **Repository** (`src/repository/event_type_repository.rs`, `_announcement_type_repository.rs`, `_membership_type_repository.rs`): three traits, three Sqlite implementations, ~250 lines each. Event and announcement diverge from each other only in table name (`event_types` vs `announcement_types`) and the usage-FK (`events.event_type_id` vs `announcements.announcement_type_id`). Membership adds the fee/period columns.
- **Service** (`src/service/event_type_service.rs`, `_announcement_type_service.rs`, `_membership_type_service.rs`): three structs with near-identical CRUD + validation logic. The `validate_hex_color` block is literally copy-pasted between event and announcement; the `count_usage > 0` delete-guard differs only in the error message.
- **Admin handlers** (`src/web/portal/admin/types.rs`, 587 lines): the event-type and announcement-type handler sets are mechanically parallel; the only differences are template paths and which service is called.

The cost shows up whenever any one of these patterns needs a fix — duplicate slug detection, sort-order policy, delete-in-use behavior, color validation. Today a fix has to land in two or three places. The risk is real but not acute: it's the kind of duplication that makes future changes feel disproportionately heavy.

The right move is *not* a sweeping generic abstraction. Event and announcement types are genuinely identical and benefit from being unified. Membership types have real divergence (a different shape, different validation, a different FK semantics) and benefit from staying separate. Squeezing all three under one generic would cost more in Rust trait ergonomics than it saves.

## What Changes

- **Unify event-type and announcement-type domain structs** into a single shape:
  - Add a `BasicTypeKind` enum (`Event`, `Announcement`) and a single `BasicType` struct holding the shared fields. Existing `EventTypeConfig`/`AnnouncementTypeConfig` become type aliases or are replaced by `BasicType` + kind-tagging where it matters at API boundaries.
  - `CreateBasicTypeRequest` / `UpdateBasicTypeRequest` replace the four parallel request structs.
- **Unify the event-type and announcement-type repositories** into a single `BasicTypeRepository` trait + `SqliteBasicTypeRepository` impl. The repository carries a `BasicTypeKind` value that selects the right table (`event_types` / `announcement_types`) and the right usage join (`events.event_type_id` / `announcements.announcement_type_id`). SQL strings are built from `&'static str` table-name constants — safe because they're internal constants, not user input.
- **Unify the event-type and announcement-type services** into a single `BasicTypeService` parameterized by `BasicTypeKind`. The shared validation (color hex format, unique slug check, delete-if-in-use guard with kind-aware error message) lives once.
- **Extract shared validators** (`validate_hex_color_for_request`, `check_unique_slug_for`, `check_unused_for_delete`) so the membership-type service can call them too. These become small private helpers in `src/service/configurable_types.rs` (new module) or top-of-file helpers in the new basic-type service.
- **Keep `MembershipTypeService` and `MembershipTypeRepository`** as-is structurally (membership has fee/period fields and FK pointing at `members.membership_type_id`). Refactor it to call the shared validator helpers instead of inlining them.
- **Collapse the admin handlers** in `src/web/portal/admin/types.rs`: replace the parallel event-type and announcement-type handler sets with a single set parameterized by `BasicTypeKind`. The "list all types" landing page (`/portal/admin/types`) and membership-specific handlers stay; basic-type new/edit/create/update/delete handlers consolidate.
- **Templates** stay distinct (event_type_form.html, announcement_type_form.html — labels and copy differ) but receive the same context shape (`BasicTypeFormContext { kind, ... }`). Or templates merge if the only difference is "Event Type" vs "Announcement Type" labels (TBD in design).
- **AppState / ServiceContext**: the two `event_type_service` / `announcement_type_service` Arcs collapse to a single `basic_type_service: Arc<BasicTypeService>`. `membership_type_service` is unchanged.
- **No database changes**: the `event_types` and `announcement_types` tables stay separate. This is purely a Rust-side abstraction collapse.

## Capabilities

### New Capabilities

(None — this consolidates existing capabilities. No new spec file is created.)

### Modified Capabilities
- `admin-types`: handler-level change. The event-type and announcement-type handler sets share a single implementation parameterized by kind; membership-type handlers stay separate. Wire shape (URLs, form bodies, templates rendered) is unchanged.
- `domain-types`: structural change. `EventTypeConfig` and `AnnouncementTypeConfig` collapse into `BasicType` with a `BasicTypeKind` discriminator. Code that imports the old type names continues to compile via type aliases for one release cycle, then aliases are removed.

## Impact

- **Code**:
  - **Removed/collapsed**: `src/repository/event_type_repository.rs` and `src/repository/announcement_type_repository.rs` are replaced by `src/repository/basic_type_repository.rs`. `src/service/event_type_service.rs` and `_announcement_type_service.rs` are replaced by `src/service/basic_type_service.rs`. `src/domain/configurable_types.rs` shrinks. Admin handler sets in `src/web/portal/admin/types.rs` consolidate.
  - **New**: shared validator helpers (small, ~20–30 lines).
  - **Net**: ~400–500 lines reduced from the codebase across these files.
- **Wire shape**: zero change. Same URLs, same form bodies, same admin pages, same templates.
- **Tests**: existing tests should continue to pass; the public-facing service methods on `BasicTypeService::with_kind(Event)` produce identical output to the old `EventTypeService`. Add unit tests for the kind-discriminated paths (delete-error message, usage table joined, list ordering) covering both `Event` and `Announcement` instantiations.
- **Risk**: medium. The repository/service consolidation touches every caller of these services — public signup flow uses `membership_type_service` (unaffected); admin handlers, recurring-event service, and event/announcement creation all consult the basic types. Mitigation: type aliases shim old names during the rollout; the compiler enforces correctness at every call site.
- **Migration**: no database migration. Rust-side rename + collapse only.
