# member-service-layout Specification

## Purpose
TBD - created by archiving change a27-split-member-service. Update Purpose after archive.
## Requirements
### Requirement: MemberService is a module directory with per-concern submodules

`src/service/member_service.rs` SHALL be converted to a module directory at `src/service/member_service/` containing the following submodules:

- `mod.rs` — `MemberService` struct, `new()` constructor, shared imports/re-exports
- `status.rs` — `activate`, `suspend`, `expire_now`
- `dues.rs` — `extend_dues`, `set_dues`
- `updates.rs` — `update`, `update_discord_id`, `resend_verification`
- `create.rs` — `create`, `bulk_import`, `send_welcome_email`
- `queries.rs` — `audit_export`, `membership_type_name`
- `events.rs` — `dispatch_member_updated` (private helper)

Each submodule's methods live in `impl MemberService { ... }` blocks. The public API of `MemberService` SHALL be identical to the pre-split file.

The autocoder MAY adjust the grouping if a different cut reads cleaner (e.g., extracting `bulk_import` to its own file given its 315-line size) as long as the size constraint below is satisfied.

#### Scenario: No submodule exceeds 400 lines

- **WHEN** the post-split files in `src/service/member_service/` are line-counted
- **THEN** every file SHALL be ≤400 lines including tests

#### Scenario: Public API is unchanged

- **WHEN** the repo is built after the split
- **THEN** every existing caller of `MemberService` SHALL compile without modification; no public method is removed, renamed, or has its visibility narrowed

#### Scenario: Tests remain co-located with their methods

- **WHEN** the tests for `activate`/`suspend`/`expire_now` are located after the split
- **THEN** they SHALL live in `status.rs` (or whichever submodule houses those methods), NOT in a separate global test file

#### Scenario: Existing tests still pass

- **WHEN** `cargo test --features test-utils` is run after the split
- **THEN** all tests that passed before SHALL still pass; no tests are lost or added by this refactor

