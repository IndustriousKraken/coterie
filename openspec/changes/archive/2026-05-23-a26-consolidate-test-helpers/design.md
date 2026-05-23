## Context

Rust's integration test layout: each `.rs` file directly under `tests/` is compiled as its own test binary. There's no implicit way to share code across them. The standard idiom for sharing is `tests/common/mod.rs` — placing the shared module in a subdirectory prevents Cargo from trying to compile it as a standalone test binary. Each test file then opts in via `mod common;`.

This is a well-known pattern; the Rust Book documents it. The reason it hasn't been adopted in coterie yet is just historical accretion — early tests defined their own helpers, later tests copy-pasted, and nobody stopped to consolidate.

## Goals / Non-Goals

**Goals:**
- Single source of truth for `fresh_pool()`, `build_app_state()`, `build_harness()`, `make_member()`, and any other helper duplicated ≥3x.
- All existing tests continue to pass with no semantic change.
- No new dependencies beyond what tests already use.

**Non-Goals:**
- Refactoring tests themselves (changing what they assert, splitting them, etc.).
- Consolidating helpers that genuinely differ in behavior across files (e.g., one `fresh_pool` seeds different baseline data than another) — those should be preserved as separately-named functions.
- Touching production code.

## Decisions

### D1. `tests/common/mod.rs` is the file, not `tests/common.rs`

The subdirectory form prevents Cargo from compiling it as a test binary. `tests/common.rs` would compile as its own (empty) test binary, which is wasteful and produces a warning. `tests/common/mod.rs` is the canonical form.

### D2. Each helper is consolidated only if all copies agree

Procedure for each helper:
1. Find all copies (via grep on the function signature).
2. Diff them.
3. If identical: move one copy to `tests/common/mod.rs`, delete the others.
4. If they differ in trivial ways (formatting, variable naming): pick the cleanest, document the reconciliation in the commit message.
5. If they differ in behaviorally significant ways (different pool options, different seed data, different return types): split into appropriately-named variants in `common::` — e.g., `fresh_pool()` and `fresh_pool_with_seed_data()`.

The autocoder MUST NOT silently merge behaviorally-different copies. Each consolidation is its own discrete step with a brief justification in the commit.

### D3. Feature-gated test utilities stay where they are

Some test code is gated behind `#[cfg(feature = "test-utils")]` because it depends on production-side test-support code (e.g., `FakeStripeGateway`). The helpers being consolidated here are pure test-side scaffolding — pool setup, harness construction. If a helper turns out to depend on a feature-gated symbol, leave it where it is rather than moving it to `common::` (which is unconditionally compiled).

### D4. Visibility: `pub(crate)` inside `common`

`tests/common/mod.rs` items are `pub` (Rust integration tests compile `common` as part of each test binary; `pub` is fine and idiomatic).

## Risks / Trade-offs

- **Risk**: a "minor" formatting difference between two copies turns out to mask a real difference (e.g., one had a `PRAGMA foreign_keys=ON` line the other lacked). → Mitigation: D2's explicit diff step. Don't merge without confirming.
- **Risk**: `tests/common/mod.rs` becomes a kitchen sink that everyone dumps random helpers into. → Mitigation: only consolidate helpers duplicated ≥3x or with strong cross-test value. Single-test helpers stay local.
- **Trade-off**: every test file gains a `mod common;` line. Trivial cost; standard Rust idiom.

## Migration Plan

Single PR.

1. Create `tests/common/mod.rs` with stub.
2. For each helper in order of duplication count (highest first — `fresh_pool` at 18×):
   - Inventory copies.
   - Diff them, reconcile per D2.
   - Add the canonical version to `common::`.
   - Replace each copy with `mod common;` + `use common::*;` and remove the local definition.
   - Run `cargo test` after each helper's consolidation to catch regressions early.
3. Once consolidated, search for any remaining `mod common;`-eligible helpers that turned up only after the initial pass.
4. `cargo test --features test-utils` — full suite passes.
