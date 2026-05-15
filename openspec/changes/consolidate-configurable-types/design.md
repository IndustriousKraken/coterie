## Context

The configurable-types subsystem has three near-parallel implementations spread across four layers:

```
                       Domain                  Repository              Service                Admin Handlers
                       ──────                  ──────────              ───────                ──────────────
event-type        EventTypeConfig      Sqlite EventTypeRepo    EventTypeService     event-type-* handlers
announcement-type AnnouncementType*    Sqlite AnnouncTypeRepo  AnnouncTypeService   announcement-type-* handlers
membership-type   MembershipType*      Sqlite MembershipRepo   MembershipTypeSvc    membership-type-* handlers
                  (fee, period extras) (extra columns)         (extra validation)   (slightly different form)
```

Two distinct kinds of duplication exist:

1. **True duplicates**: event-type and announcement-type are identical at the domain, repository, and service layers. The only differences are:
   - Table name: `event_types` vs. `announcement_types`
   - Usage-FK target: `events.event_type_id` vs. `announcements.announcement_type_id`
   - Error-message strings: "event type" vs. "announcement type"
2. **Genuine divergence**: membership-type has additional fields (`fee_cents`, `billing_period`), additional validation (billing-period parse, fee non-negative), a different usage-FK target (`members.membership_type_id`), and is consulted from non-admin call sites (signup, recurring billing).

The right scope is to collapse the true duplicates and keep the divergent type separate, while extracting just the *shared validation logic* so all three services use one canonical hex-color check, one unique-slug check, and one delete-if-in-use check.

## Goals / Non-Goals

**Goals:**
- A single Rust-side implementation for event-type and announcement-type behavior, parameterized by a `BasicTypeKind` discriminator.
- All three configurable types use one canonical implementation of the shared validators (color, slug uniqueness, delete-in-use).
- Wire shape unchanged: same URLs, same templates rendered, same form bodies, same SQL outcomes.
- Public domain names (`EventTypeConfig`, `AnnouncementTypeConfig`) remain referenceable during the transition via type aliases so non-trivial call sites can migrate without churn.
- Net reduction in code surface; net increase in change-locality (one fix lands in one place).

**Non-Goals:**
- Subsuming `MembershipTypeService` into the same generic. Membership has real shape divergence; forcing it into the same machinery would either constrain it or pollute the basic-type abstraction with optional fields that don't apply.
- Database changes. The three tables (`event_types`, `announcement_types`, `membership_types`) stay separate; row schemas don't change.
- Eliminating the kind-tagged error strings ("Cannot delete event type:" vs. "Cannot delete announcement type:"). Those land via the discriminator and a small `display_name()` method on the kind enum.
- Trait-level genericism (`ConfigurableTypeService<T: ConfigurableType>`). The trait approach was considered and rejected (see D2).
- Templating consolidation. Whether `event_type_form.html` and `announcement_type_form.html` merge into one template is a separate, smaller question; this change preserves the existing templates and feeds them via a parameterized context.

## Decisions

### D1. `BasicTypeKind` is a small enum with const data

```rust
#[derive(Debug, Clone, Copy)]
pub enum BasicTypeKind { Event, Announcement }

impl BasicTypeKind {
    pub fn table(self) -> &'static str { match self { Event => "event_types", Announcement => "announcement_types" } }
    pub fn usage_table(self) -> &'static str { match self { Event => "events", Announcement => "announcements" } }
    pub fn usage_fk(self) -> &'static str { match self { Event => "event_type_id", Announcement => "announcement_type_id" } }
    pub fn display_name(self) -> &'static str { match self { Event => "event type", Announcement => "announcement type" } }
}
```

The kind is `Copy` and accessor methods return `&'static str`. SQL strings interpolate these constants safely — they're not user-controlled, and the compiler enforces every variant-returning helper is total.

### D2. One concrete service, not a generic trait

Considered: `trait ConfigurableType { type Config; type Repo; ... }` with `EventType` and `AnnouncementType` as zero-sized type-level marker types implementing the trait, and a single generic service `ConfigurableTypeService<T: ConfigurableType>`. Rejected:
- Rust generics over async trait methods still require boilerplate (`async_trait`, ceremony for object-safety vs. monomorphization).
- The discriminator only varies in four `&'static str` values; an enum captures that more honestly than a type-level encoding.
- "Read code and understand what it does" is easier with `match kind { Event => ... }` than with monomorphized phantom types.

The chosen shape: a single `BasicTypeService` that holds an `Arc<dyn BasicTypeRepository>` and a `BasicTypeKind`. The repo is constructed once with a `kind` baked in; alternatively the kind is passed on every call. We pick the per-instance baking pattern (D3).

### D3. The kind is baked into the service instance, constructed twice in `ServiceContext`

```rust
let basic_type_repo: Arc<dyn BasicTypeRepository> =
    Arc::new(SqliteBasicTypeRepository::new(db_pool.clone()));

let event_type_service = Arc::new(BasicTypeService::new(basic_type_repo.clone(), BasicTypeKind::Event));
let announcement_type_service = Arc::new(BasicTypeService::new(basic_type_repo.clone(), BasicTypeKind::Announcement));
```

Two service instances share one repository, each carrying its own kind. Callers say `state.service_context.event_type_service.create(...)` exactly as before — no extra parameter at the call site, because the kind is already in the service instance.

The repository methods take `kind: BasicTypeKind` as a parameter on every call (the repo doesn't bake the kind, since the same SqliteBasicTypeRepository serves both kinds). The service forwards its baked-in kind to every repo call.

### D4. `BasicType` is a single domain struct; old names become type aliases

```rust
pub struct BasicType {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub sort_order: i32,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub type EventTypeConfig = BasicType;
pub type AnnouncementTypeConfig = BasicType;
```

Type aliases preserve compile-compatibility for existing call sites during the migration. After the change lands, the aliases stay (no rush to remove) — they cost nothing and keep the domain language familiar at the API boundary. CLAUDE.md says no backwards-compat shims for unused things, but these aliases are *used* and serve as kind-of-documentation at the call site ("this function returns event-type-flavored data").

`CreateBasicTypeRequest` / `UpdateBasicTypeRequest` replace the four parallel request structs. The old names get aliases too.

### D5. Shared validators live in `src/service/configurable_types.rs` (new module)

Three small helpers:

```rust
pub(crate) fn validate_hex_color_for_request(color: Option<&str>) -> Result<()>;
pub(crate) async fn check_unique_slug_for_basic(
    repo: &dyn BasicTypeRepository, kind: BasicTypeKind, slug: &str,
) -> Result<()>;
pub(crate) async fn check_delete_unused_for_basic(
    repo: &dyn BasicTypeRepository, kind: BasicTypeKind, id: Uuid, display: &str,
) -> Result<()>;
```

`MembershipTypeService` uses `validate_hex_color_for_request` (the slug and delete checks have membership-specific shapes against the membership repo). The basic-type service uses all three.

### D6. SQL strings interpolate `kind.table()` directly

```rust
let q = format!(
    "SELECT id, name, slug, ... FROM {} WHERE id = ?",
    kind.table(),
);
sqlx::query_as::<_, BasicTypeRow>(&q).bind(...).fetch_optional(...).await
```

Considered: macro-generated SQL per kind. Rejected — `format!` over compile-time-constant `&'static str` is just as safe, simpler to read, and avoids the macro-debugging cost. The interpolated value is never user-controlled.

A small risk: prepared-statement caching keys by string identity, so each kind builds its own cache slot per query. Two extra cache entries per query × ~6 queries ≈ 12 extra prepared statements. Negligible memory impact for a single-tenant app on a SQLite pool of ~10 connections.

### D7. Admin handlers parameterize by kind

The current admin handlers in `src/web/portal/admin/types.rs` look like this for event types:

```rust
admin_new_event_type_page         // form GET
admin_create_event_type           // form POST
admin_edit_event_type_page        // detail GET
admin_update_event_type           // detail POST
admin_delete_event_type           // delete POST
```

There's a parallel set for announcement types. After the change there's one set, parameterized by kind from the URL path:

```
/portal/admin/types/:kind/new       (kind ∈ {event, announcement})
/portal/admin/types/:kind/:id
/portal/admin/types/:kind/:id/delete
```

…or alternatively the kind is hardcoded in the route registration:

```rust
.route("/types/event/new",         get(basic_types::new_page))
.route("/types/announcement/new",  get(basic_types::new_page))
```

with the handler reading the kind from a `Path` extractor. Choosing between these is a minor decision in implementation; either preserves the wire shape exactly. Membership-type routes are unchanged (`/types/membership/...` stays).

### D8. Templates stay separate but receive a uniform context

`event_type_form.html` and `announcement_type_form.html` look nearly identical today (form fields are the same; labels and the `<form action="">` URL differ). They stay as two files initially; the handler builds one `BasicTypeFormContext { kind, type_data, is_edit }` and Askama picks the template by kind.

A follow-up change can decide whether to merge the two templates into one parameterized by `kind.display_name()`. That's separable from this refactor.

### D9. Membership type stays separate at every layer

`MembershipType` keeps its own domain struct, repository trait, repository impl, service struct, and admin handlers. The only change to the membership side: it imports `validate_hex_color_for_request` from the shared module instead of inlining it.

### D10. Migration is single-PR, with type aliases bridging old call sites

Type aliases (`pub type EventTypeConfig = BasicType;`) mean every existing reference to `EventTypeConfig` keeps compiling. The change lands in one PR; there's no two-step alias-then-remove workflow.

## Risks / Trade-offs

- **Risk**: SQL injection via the `kind.table()` interpolation if a future contributor adds user-controllable input to `BasicTypeKind`. → **Mitigation**: keep `BasicTypeKind` a closed enum with const accessors only. Document the constraint at the kind-definition site.
- **Risk**: prepared-statement cache duplication (one slot per kind per query). → Trivial impact for this codebase scale; not worth defending against.
- **Risk**: behavior drift during the consolidation (e.g., an off-by-one in sort-order assignment). → **Mitigation**: existing tests assert behavior for at least event types; add the announcement-type counterparts before/after.
- **Trade-off**: the kind discriminator slightly increases per-call overhead (a `match` on three `&'static str` lookups per repo call). Compiler likely folds it; not measurable.
- **Trade-off**: the abstraction's "blast radius" is bigger than the refactor's complexity warrants if you only count lines. The win is in change-locality — *future* edits to color validation, slug uniqueness, or delete-in-use behavior land once.
- **Trade-off**: type aliases keep `EventTypeConfig` as a name in the codebase indefinitely. CLAUDE.md prefers no backwards-compat shims, but these aren't shims for removed code — they're concise names for a kind-flavored view of `BasicType`. Keeping them is a deliberate readability choice, not a rollout artifact.

## Migration Plan

Single PR; pure-internal refactor.

1. Add `BasicType`, `BasicTypeKind`, `CreateBasicTypeRequest`, `UpdateBasicTypeRequest` in `src/domain/configurable_types.rs`. Add type aliases for the old names.
2. Create `src/service/configurable_types.rs` (shared validator helpers).
3. Create `src/repository/basic_type_repository.rs` with `BasicTypeRepository` trait and `SqliteBasicTypeRepository` impl that takes `kind` per-call.
4. Create `src/service/basic_type_service.rs` with `BasicTypeService` baking `kind`. Make existing service-method names match (e.g., `BasicTypeService::list`, `::get`, `::create`, `::update`, `::delete`).
5. Update `ServiceContext::new` to construct two `BasicTypeService` instances (one per kind) sharing one repo. Continue exposing them as `event_type_service` and `announcement_type_service` so call sites don't move.
6. Refactor `MembershipTypeService` to call the shared `validate_hex_color_for_request` helper.
7. Delete `src/repository/event_type_repository.rs`, `_announcement_type_repository.rs`, `src/service/event_type_service.rs`, `_announcement_type_service.rs`. Compiler will catch any remaining references.
8. Consolidate the event-type and announcement-type admin handlers in `src/web/portal/admin/types.rs` into a single kind-parameterized set.
9. Run `cargo test --features test-utils`. Existing tests pass; add new tests for the kind-tagged paths (delete error message includes the right display name; list query targets the right table).
10. Deploy normally. No DB migrations, no config changes, no flags.
