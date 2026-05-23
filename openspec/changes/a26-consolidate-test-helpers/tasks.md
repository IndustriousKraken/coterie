## 1. Scaffold

- [ ] 1.1 Create `tests/common/mod.rs`. Add a brief module doc comment.
- [ ] 1.2 Confirm Cargo doesn't try to compile `tests/common/mod.rs` as its own test binary (it shouldn't, because it's in a subdirectory — but verify with `cargo test --no-run` and check the test-binary list).

## 2. Consolidate fresh_pool (18× duplication)

- [ ] 2.1 `grep -rn "fn fresh_pool" tests/` to list every copy.
- [ ] 2.2 Diff all copies. If identical: pick one, move to `common::fresh_pool()`. If they differ trivially (whitespace, variable names): reconcile to one canonical form. If they differ behaviorally: split into named variants per spec.
- [ ] 2.3 For each file that had a local copy: add `mod common;` near the top, replace `fresh_pool()` calls with `common::fresh_pool()` (or `use common::fresh_pool;`), delete the local definition.
- [ ] 2.4 `cargo test --features test-utils` — all tests still pass.

## 3. Consolidate build_harness (6×)

- [ ] 3.1 Same procedure as section 2: inventory, diff, reconcile, move to `common::`, update call sites, delete local copies.
- [ ] 3.2 `cargo test --features test-utils`.

## 4. Consolidate build_app_state (3×)

- [ ] 4.1 Same procedure.
- [ ] 4.2 `cargo test --features test-utils`.

## 5. Consolidate make_member (3×)

- [ ] 5.1 Same procedure. Note: make_member exists in both test files (`tests/billing_dashboard_test.rs:45`) and inside `src/service/member_service.rs:1021` as a test helper inside `#[cfg(test)]`. The latter is a unit-test helper in a different scope and SHALL NOT be moved to `tests/common/mod.rs` (it's not an integration test).
- [ ] 5.2 `cargo test --features test-utils`.

## 6. Consolidate build (2×) — only if implementations agree

- [ ] 6.1 The architecture finding is `build()` across `tests/recurring_event_test.rs:119` and one other test file. Inspect both. If they're the same builder pattern, move to `common::`. If they're different builder types (`EventBuilder::build()` vs `MemberBuilder::build()`), they're NOT duplicates — skip this consolidation.
- [ ] 6.2 `cargo test --features test-utils`.

## 7. Consolidate count (2×)

- [ ] 7.1 The findings are `count(&self)` across two test files. If it's a row-count helper, move to `common::`. If they're different (one counts members, one counts something else), skip.
- [ ] 7.2 `cargo test --features test-utils`.

## 8. Sweep for any remaining duplicates

- [ ] 8.1 Re-run the duplicate-signature scan (or grep for any `fn ` definition appearing in ≥3 test files). Any new duplicates that surface — consolidate using the same procedure.

## 9. Validation

- [ ] 9.1 `cargo test --features test-utils` — full suite passes.
- [ ] 9.2 `cargo clippy --tests -- --deny warnings` — clean.
- [ ] 9.3 `cargo fmt --check` — clean.
- [ ] 9.4 Confirm test count is unchanged: `cargo test --features test-utils 2>&1 | grep "test result" | tail -1` before and after this change should report the same total test count.
