## ADDED Requirements

### Requirement: BasicType collapses event-type and announcement-type into one struct

`domain::BasicType` SHALL be a single Rust struct holding the fields shared by event types and announcement types: `id`, `name`, `slug`, `description`, `color`, `icon`, `sort_order`, `is_active`, `created_at`, `updated_at`. The kind discriminator (`BasicTypeKind`) SHALL NOT be stored on the struct itself — it lives on the service / repository / handler that produced or consumes the value, because the two type lists are physically separate tables.

`EventTypeConfig` and `AnnouncementTypeConfig` SHALL become type aliases for `BasicType` so existing call sites continue to compile and read naturally at the API boundary.

#### Scenario: BasicType has no kind field on the row

- **WHEN** code reads a `BasicType` value
- **THEN** the value SHALL NOT carry a kind discriminator on the struct itself; the kind is implicit in which service/repository the value came from

#### Scenario: Old type names continue to be importable

- **WHEN** existing code imports `EventTypeConfig` or `AnnouncementTypeConfig`
- **THEN** the import SHALL continue to resolve via type aliases and SHALL refer to the same `BasicType` underneath

### Requirement: Request shapes unify with type aliases

`CreateBasicTypeRequest` and `UpdateBasicTypeRequest` SHALL replace the four parallel request structs (`CreateEventTypeRequest`, `CreateAnnouncementTypeRequest`, `UpdateEventTypeRequest`, `UpdateAnnouncementTypeRequest`). The old names SHALL remain as type aliases.

`MembershipType`'s request shapes SHALL stay separate — they carry `fee_cents` and `billing_period` fields not present on the basic shape.

#### Scenario: Old request type names continue to be importable

- **WHEN** existing code references `CreateEventTypeRequest` or `UpdateAnnouncementTypeRequest`
- **THEN** the reference SHALL resolve via type alias to the unified `CreateBasicTypeRequest` / `UpdateBasicTypeRequest`

### Requirement: BasicTypeKind is a closed enum with const accessors

`domain::BasicTypeKind` SHALL be a closed Rust enum with variants `Event` and `Announcement`. The enum SHALL expose const-equivalent accessors (`table()`, `usage_table()`, `usage_fk()`, `display_name()`) returning `&'static str` so SQL strings and error messages can be built without runtime branching at every call site.

The kind SHALL NOT be extended to admit user-controlled values. Adding a new variant SHALL force every accessor to return a value for it (the compiler enforces totality on the `match` expressions inside the accessors).

#### Scenario: SQL strings interpolate kind.table() safely

- **WHEN** the basic-type repository builds a SQL statement
- **THEN** it SHALL interpolate the `&'static str` from `kind.table()` (and similar accessors); the value SHALL NOT come from user input or runtime configuration

#### Scenario: Adding a new kind forces every accessor to be updated

- **WHEN** a contributor adds a new `BasicTypeKind` variant
- **THEN** the compiler SHALL fail to build until every const accessor (`table`, `usage_table`, `usage_fk`, `display_name`) returns a value for the new variant
