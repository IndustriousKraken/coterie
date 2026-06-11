# repository-contracts Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Repository methods are declared on a trait

Each repository SHALL have a trait declared in its per-file module (e.g., `MemberRepository` in `src/repository/member_repository.rs`) alongside the matching Sqlite implementation. The trait SHALL be re-exported from `src/repository/mod.rs` via `pub use` so external callers can continue to import it as `crate::repository::<TraitName>`.

`src/repository/mod.rs` SHALL contain only `pub mod` declarations and `pub use` re-exports — no trait declarations, no impl blocks, no auxiliary types. Auxiliary types (e.g., `MemberQuery`, `MemberSortField`, `SortOrder`, `MonthlyRevenue`) SHALL live in the same per-file module as the trait that uses them.

New repository methods SHALL be added to the trait, not just the impl, so callers depend on the trait.

#### Scenario: New method is added to trait first, in the per-file module

- **WHEN** a contributor adds a new repository method
- **THEN** the method SHALL be declared on the trait (located in the per-file module, e.g., `src/repository/member_repository.rs`) and implemented in the impl in the same file; tests and callers SHALL hold the trait-typed reference

#### Scenario: mod.rs is a module index, not a trait dump

- **WHEN** a contributor inspects `src/repository/mod.rs`
- **THEN** the file SHALL contain only `pub mod <module>;` declarations and `pub use <module>::{...};` re-exports; it SHALL NOT contain any `pub trait`, `pub struct`, `pub enum`, or `impl` block

#### Scenario: Tests can substitute a fake impl

- **WHEN** a test wants to substitute a repository
- **THEN** the consumer SHALL hold a trait object (or generic) so a fake or in-memory impl can be substituted; the trait import path (`use crate::repository::<TraitName>;`) SHALL continue to resolve via the re-export

#### Scenario: Auxiliary types live next to the trait that uses them

- **WHEN** a contributor inspects `MemberQuery`, `MemberSortField`, `SortOrder`
- **THEN** they SHALL be defined in `src/repository/member_repository.rs` alongside `MemberRepository`; they SHALL be re-exported from `mod.rs` so `crate::repository::MemberQuery` continues to resolve

#### Scenario: Adding a new repository follows the per-file pattern

- **WHEN** a contributor adds a new repository (e.g., for a new entity)
- **THEN** the trait, the Sqlite impl, the row struct, and any auxiliary types SHALL all live in a single `<entity>_repository.rs` file; `mod.rs` SHALL gain a `pub mod` line and a `pub use` line, nothing more

### Requirement: Strongly-typed query/sort inputs prevent stringly-typed mistakes

Repository search methods SHALL accept strongly-typed query objects (e.g., `MemberQuery` with `MemberSortField` and `SortOrder` enums) rather than free-form strings. The trait surface SHALL preclude an unknown sort field reaching the impl, and the impl SHALL map enum variants to column names in one place — eliminating SQL-injection risk on the sort path.

#### Scenario: Caller cannot pass an unknown sort field

- **WHEN** a handler builds a `MemberQuery`
- **THEN** the `sort` field SHALL be a `MemberSortField` enum variant; passing a string is impossible at the type level

### Requirement: Idempotency and conflict semantics documented where they matter

Repository trait methods that have non-trivial concurrency or conflict behavior (e.g., `ProcessedEventsRepository::claim` returning bool to signal "already claimed", `list_with_discord_id` cross-status semantics) SHALL document the contract on the trait. Trivial CRUD methods MAY rely on convention (`create` returns the created row; `find_by_*` returns `Option`; `update` returns the updated row).

#### Scenario: Non-trivial methods document their contract

- **WHEN** a contributor reads `ProcessedEventsRepository::claim` on the trait
- **THEN** the doc comment SHALL describe atomic insert-or-skip semantics with a `bool` return for "claimed vs already-claimed"

#### Scenario: Trivial CRUD methods rely on convention

- **WHEN** a contributor reads a `find_by_*` method
- **THEN** the absence of a doc comment SHALL be acceptable because the convention (returns `Option`, `None` = not found) is universal across repositories

### Requirement: Repo tests use in-memory SQLite, not mocks

Repository tests SHALL use an in-memory SQLite database (the pattern in `tests/billing_dashboard_test.rs`). Mocking the database SHALL be avoided so tests catch real schema / migration drift.

#### Scenario: Repo test boots a real schema

- **WHEN** a new repo test is added
- **THEN** it SHALL set up an in-memory SQLite database with the production migrations and exercise the real impl

