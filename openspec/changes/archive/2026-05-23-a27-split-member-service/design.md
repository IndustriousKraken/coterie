## Context

`src/service/member_service.rs` accumulated to 1262 lines over time. The first `impl MemberService` block (line 101) is 750 lines of methods covering everything member-shaped that's an admin action. Tests start around line 970.

Function inventory (from `grep` at spec-writing time):

| Line | Function | Lines (approx) | Concern |
|------|----------|---------------|---------|
| 133  | `activate` | 46 | status |
| 179  | `suspend` | 49 | status |
| 228  | `update` | 36 | profile updates |
| 264  | `extend_dues` | 46 | dues |
| 310  | `set_dues` | 36 | dues |
| 346  | `expire_now` | 30 | status |
| 376  | `update_discord_id` | 41 | profile updates |
| 417  | `resend_verification` | 82 | profile updates |
| 499  | `create` | 40 | creation |
| 539  | `bulk_import` | 315 | creation (the big one) |
| 854  | `send_welcome_email` | 52 | creation helper (private) |
| 906  | `dispatch_member_updated` | 23 | events helper (private) |
| 929  | `audit_export` | 27 | queries |
| 956  | `membership_type_name` | 20 | queries |

`bulk_import` at 315 lines is the single biggest function. The other concerns are 100–250 lines each.

## Goals / Non-Goals

**Goals:**
- No file in the new layout exceeds ~400 lines including tests.
- Each submodule is a coherent concern that a reviewer can hold in their head.
- Public API of `MemberService` is unchanged — every existing caller compiles and runs without modification.

**Non-Goals:**
- Changing any function's behavior, signature, or visibility.
- Refactoring `bulk_import` itself (that's a follow-up if anyone wants it).
- Touching `MemberServiceTrait` or repository code.

## Decisions

### D1. Module directory, not flat-file split

Rust supports `src/service/member_service.rs` OR `src/service/member_service/mod.rs`. The latter lets us add sibling files (`status.rs`, `dues.rs`, etc.). All submodules' `impl MemberService` blocks operate on the same struct (Rust allows multiple `impl` blocks for the same type across different files).

### D2. Submodule layout (proposed; autocoder may adjust)

```
src/service/member_service/
├── mod.rs           — struct definition, new(), re-exports, common imports
├── status.rs        — activate, suspend, expire_now + their tests
├── dues.rs          — extend_dues, set_dues + their tests
├── updates.rs       — update, update_discord_id, resend_verification + their tests
├── create.rs        — create, bulk_import, send_welcome_email + their tests
├── queries.rs       — audit_export, membership_type_name + their tests
└── events.rs        — dispatch_member_updated (private helper used across files)
```

Rationale:
- Status/dues/updates split by concern (these are the three things an admin does to an existing member).
- `create.rs` holds both `create` (one-off) and `bulk_import` (many-at-once) since they share `send_welcome_email`. ~410 lines including tests — still under the 800 threshold.
- `queries.rs` is small read-only helpers.
- `events.rs` holds the private dispatch helper that's called from multiple other files.

If the autocoder finds a cleaner grouping during implementation (e.g., `bulk_import` warrants its own file because it's 315 lines on its own), they MAY adjust. The requirement is "no file >400 lines" — the exact partitioning is implementation choice.

### D3. Tests move with their methods

Each submodule gets its own `#[cfg(test)] mod tests { ... }` block at the bottom containing the tests that exercise that submodule's methods. The shared test-helpers (`fresh_pool`, `make_service`, `make_member`, `audit_count`) need either:
- A shared `src/service/member_service/test_helpers.rs` file with `#[cfg(test)] pub(super) fn ...`, OR
- A small `pub(super) mod test_helpers` inside `mod.rs`.

Pick whichever's cleaner. Calling from sibling test modules via `use super::test_helpers::*` or `use crate::service::member_service::test_helpers::*` is the access pattern.

Note: these are unit tests inside the service module, NOT integration tests. They are NOT affected by `a26` (which targets `tests/common/`).

### D4. No visibility changes

Every function that was `pub` stays `pub`. Every function that was `async fn` (private) stays private. The split is purely cosmetic from the outside.

## Risks / Trade-offs

- **Risk**: an accidental `pub(crate)` → `pub(super)` change breaks a caller. → Mitigation: `cargo build` (with all features) catches this.
- **Risk**: tests reference shared helpers (`make_service`, `fresh_pool`) by path; reorganizing breaks the references. → Mitigation: D3's explicit test-helpers placement.
- **Risk**: future PRs add new methods and the cleanly-split layout drifts back to a kitchen sink. → Mitigation: not in scope for this change. Document the split intent in `mod.rs`'s doc comment so future contributors place new methods in the right submodule.
- **Trade-off**: more files. Worth it given the current 1262-line monolith.

## Migration Plan

Single PR.

1. Create the new module directory + empty submodules.
2. Move struct + `new()` to `mod.rs`.
3. Move each method group to its assigned submodule. Move tests at the same time.
4. Reconcile imports — each submodule needs its own `use` lines.
5. `cargo build --features test-utils` — clean compile.
6. `cargo test --features test-utils` — all tests pass.
7. `cargo clippy --features test-utils -- --deny warnings` — clean.
