## Why

The architecture pass surfaced 18 copies of `fresh_pool()`, 6 of `build_harness()`, 3 of `build_app_state(pool)`, 3 of `make_member(pool)`, plus a handful of other duplicated setup functions across `tests/*.rs`. Each test file has been growing its own scaffolding because Rust's integration-test layout (one binary per `.rs` file in `tests/`) makes sharing awkward by default. The duplicates drift — three different copies of `fresh_pool` will eventually disagree on migrations, pragmas, or pool options, and the next test failure becomes a half-hour mystery about why one test sees a different schema.

The standard Rust fix is `tests/common/mod.rs` — a `mod.rs` placed in a subdirectory so it doesn't compile as its own test binary, included from each test file with `mod common;`.

## What Changes

- **New `tests/common/mod.rs`** containing the canonical implementations of the duplicated helpers:
  - `fresh_pool()` — fresh in-memory SQLite pool with all migrations applied.
  - `build_app_state(pool)` — produces the `AppState` used by router-level tests.
  - `build_harness()` — produces whatever bundle of state + fakes router tests need.
  - `make_member(pool, ...)` — inserts a test member, returns the `Member` domain object.
  - `build()` — generic builder pattern shared by builder-style fixtures (if it's the same shape across files).
  - `count(pool, table)` — generic row-count helper if applicable.
- **Each duplicating test file** updated to `mod common;` + `use common::*;` and the local definition deleted.
- **No production code changes.** This is test-side scaffolding only.

## Capabilities

### New Capabilities
- `test-infrastructure`: shared scaffolding for integration tests (`fresh_pool`, `build_app_state`, `build_harness`, `make_member`, ...) lives in a single `tests/common/mod.rs` module rather than being duplicated across test files.

### Modified Capabilities
None.

## Impact

- **Code**: net negative line count (deleting ~17 copies, adding one consolidated module). `tests/common/mod.rs` is ~150–250 lines depending on how generic the helpers need to be.
- **Wire shape**: zero runtime change.
- **Tests**: `cargo test --features test-utils` SHALL continue to pass with the same set of tests, same set of assertions. The autocoder's validation gate is "all tests still pass."
- **Risk**: low. Pure mechanical refactor. The only risk is the canonical implementation accidentally differing from one of the existing copies in a way that matters — e.g., one copy enabled WAL mode and another didn't. Each consolidation step requires diffing the copies and reconciling explicitly (documented in tasks).
- **Dependency**: none. Independent of a23/a24/a25.
