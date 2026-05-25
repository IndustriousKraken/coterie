# test-infrastructure Specification

## Purpose
TBD - created by archiving change a26-consolidate-test-helpers. Update Purpose after archive.
## Requirements
### Requirement: Shared test helpers live in tests/common/mod.rs

Integration-test scaffolding functions duplicated across multiple test files SHALL be consolidated into `tests/common/mod.rs`. The following helpers (identified by the architecture pass) SHALL have exactly one canonical implementation in `tests/common/mod.rs`, with all other copies removed:

- `fresh_pool()` — currently duplicated across 18 test files
- `build_app_state(pool: SqlitePool)` — 3 files
- `build_harness()` — 6 files
- `make_member(pool: &SqlitePool, ...)` — 3 files
- `build()` — 2 files (only if the implementations agree)
- Other helpers duplicated ≥3 times that surface during the consolidation

Test files using these helpers SHALL include them via `mod common;` and `use common::*;` (or specific items) in place of local definitions.

The shared module SHALL NOT contain helpers that are genuinely test-specific or depend on `#[cfg(feature = "test-utils")]`-gated production-side code that conflicts with unconditional compilation.

#### Scenario: fresh_pool has exactly one definition

- **WHEN** the codebase is grepped for `fn fresh_pool` after this change
- **THEN** exactly one definition SHALL exist, and it SHALL live in `tests/common/mod.rs`

#### Scenario: All previously-passing tests still pass

- **WHEN** `cargo test --features test-utils` is run after consolidation
- **THEN** every test that passed before SHALL still pass; no tests are lost or added by this refactor

#### Scenario: Behaviorally-different copies are preserved as separately-named variants

- **WHEN** two copies of a helper turn out to differ in behaviorally meaningful ways (different seed data, different pool options, different return shape)
- **THEN** the consolidation SHALL preserve both as separately-named functions in `tests/common/mod.rs` (e.g., `fresh_pool()` and `fresh_pool_with_seed()`), NOT silently pick one and discard the other

