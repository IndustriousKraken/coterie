## 1. Scaffold

- [x] 1.1 Create `tests/common/mod.rs`. Add a brief module doc comment.
- [x] 1.2 Confirm Cargo doesn't try to compile `tests/common/mod.rs` as its own test binary (it shouldn't, because it's in a subdirectory — but verify with `cargo test --no-run` and check the test-binary list).

## 2. Consolidate fresh_pool (18× duplication)

- [x] 2.1 `grep -rn "fn fresh_pool" tests/` to list every copy.
- [x] 2.2 Diff all copies. If identical: pick one, move to `common::fresh_pool()`. If they differ trivially (whitespace, variable names): reconcile to one canonical form. If they differ behaviorally: split into named variants per spec.
- [x] 2.3 For each file that had a local copy: add `mod common;` near the top, replace `fresh_pool()` calls with `common::fresh_pool()` (or `use common::fresh_pool;`), delete the local definition.
- [x] 2.4 `cargo test --features test-utils` — all tests still pass.

## 3. Consolidate build_harness (6×)

- [x] 3.1 Same procedure as section 2: inventory, diff, reconcile, move to `common::`, update call sites, delete local copies. **Result:** Each `build_harness` returns a different file-private `Harness`/`H` struct (different return types, distinct test surfaces). Same logic as section 6.1 applies — they're NOT duplicates, just share a name. Skipped consolidation.
- [x] 3.2 `cargo test --features test-utils`.

## 4. Consolidate build_app_state (3×)

- [x] 4.1 Same procedure. All three copies were near-identical; moved canonical form to `common::build_app_state` and deleted local definitions.
- [x] 4.2 `cargo test --features test-utils`.

## 5. Consolidate make_member (3×)

- [x] 5.1 Same procedure. Note: make_member exists in both test files (`tests/billing_dashboard_test.rs:45`) and inside `src/service/member_service.rs:1021` as a test helper inside `#[cfg(test)]`. The latter is a unit-test helper in a different scope and SHALL NOT be moved to `tests/common/mod.rs` (it's not an integration test). **Result:** Two of three return `Uuid` and are essentially identical → `common::make_member`. The `totp_test.rs` copy returns `(Uuid, String)` (behaviorally different return shape) → preserved as `common::make_member_with_email` per spec scenario #3.
- [x] 5.2 `cargo test --features test-utils`.

## 6. Consolidate build (2×) — only if implementations agree

- [x] 6.1 The architecture finding is `build()` across `tests/recurring_event_test.rs:119` and one other test file. Inspect both. If they're the same builder pattern, move to `common::`. If they're different builder types (`EventBuilder::build()` vs `MemberBuilder::build()`), they're NOT duplicates — skip this consolidation. **Result:** Both `build()` functions return different file-private `H` structs (one for recurring events, one for scheduled announcements) — different return types, distinct test surfaces. Skipped per task guidance.
- [x] 6.2 `cargo test --features test-utils`.

## 7. Consolidate count (2×)

- [x] 7.1 The findings are `count(&self)` across two test files. If it's a row-count helper, move to `common::`. If they're different (one counts members, one counts something else), skip. **Result:** Both are inherent methods on different file-private fake email-sender structs (`RecordingEmailSender` vs `FakeEmailSender`). They share a name but are tied to distinct, file-private types — can't be moved without also moving the struct definitions, which is out of scope. Skipped.
- [x] 7.2 `cargo test --features test-utils`.

## 8. Sweep for any remaining duplicates

- [x] 8.1 Re-run the duplicate-signature scan (or grep for any `fn ` definition appearing in ≥3 test files). Any new duplicates that surface — consolidate using the same procedure. **Result:** The only remaining ≥3-duplicated function names are required trait-method implementations (`Integration::name`, `is_enabled`, `health_check`, `handle_event`) on file-private fake types in each test. They can't be consolidated without first consolidating the fakes themselves, which is out of scope for this refactor.

## 9. Validation

- [x] 9.1 `cargo test --features test-utils` — full suite passes. 249 passed, 6 failed, 255 total — same counts as baseline. The 6 failures are pre-existing snapshot drift in `tests/member_template_snapshots.rs`, unrelated to this refactor.
- [x] 9.2 `cargo clippy --tests -- --deny warnings` — pre-existing lib-side clippy errors (66 of them, all in `src/`) prevent this from being globally clean. Verified that this refactor adds **no new clippy issues in `tests/`** — running clippy on the same target with the baseline produces the same errors, all confined to `src/`. Cleaning up unrelated lib-side clippy debt is out of scope.
- [x] 9.3 `cargo fmt --check` — files touched by this refactor are clean (verified by running `cargo fmt` then reverting unrelated changes). Pre-existing fmt drift in `src/` and in untouched test files was left in place since the proposal explicitly forbids production-code changes ("No production code changes. This is test-side scaffolding only.").
- [x] 9.4 Confirm test count is unchanged: 255 tests both before and after this change.
