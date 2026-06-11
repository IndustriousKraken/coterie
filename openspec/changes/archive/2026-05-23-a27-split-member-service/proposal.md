## Why

`src/service/member_service.rs` is 1262 lines — the largest file in the repo. The single `impl MemberService` block holds 16 distinct methods covering status transitions, dues management, profile updates, bulk import, helpers, and audit-export. Plus ~285 lines of tests at the bottom. The file is harder to navigate than it needs to be, and reviewing changes to `bulk_import` (a ~315-line function on its own) means scrolling through 1000 lines of unrelated context.

Splitting `MemberService` into a module directory with per-concern submodules makes each concern a self-contained file 200–350 lines long. The public API stays identical; this is a pure code-organization refactor.

## What Changes

- Convert `src/service/member_service.rs` (single file) → `src/service/member_service/` (module directory).
- Module layout (autocoder may adjust groupings if a different cut reads cleaner — see design.md D2):
  - `mod.rs` — `MemberService` struct, `new()` constructor, re-exports
  - `status.rs` — `activate`, `suspend`, `expire_now`
  - `dues.rs` — `extend_dues`, `set_dues`
  - `updates.rs` — `update`, `update_discord_id`, `resend_verification`
  - `create.rs` — `create`, `bulk_import` (the big one), `send_welcome_email`
  - `queries.rs` — `audit_export`, `membership_type_name`
  - `events.rs` — `dispatch_member_updated` (private helper)
- Tests stay with the methods they cover (each submodule gets its own `#[cfg(test)] mod tests` block at the bottom).
- All `impl MemberService` blocks across the new files use the same struct; Rust allows multiple `impl` blocks for the same type, no API change.
- No callers of `MemberService` need to change.

## Capabilities

### New Capabilities
- `member-service-layout`: `MemberService` is organized as a module directory with per-concern submodules (status, dues, updates, create, queries, events), no submodule file exceeds ~400 lines.

### Modified Capabilities
None.

## Impact

- **Code**: net-neutral line count (same code, different file partitioning).
- **Wire shape**: zero change.
- **Tests**: existing tests pass unchanged. Each test moves with the function under test.
- **Risk**: low. Mechanical refactor. Risks: missing a `use` import after the split (caught by `cargo build`); accidentally changing a function's visibility (caught by `cargo build` from callers).
- **Dependency**: none. Independent of a23/a24/a25/a26.
