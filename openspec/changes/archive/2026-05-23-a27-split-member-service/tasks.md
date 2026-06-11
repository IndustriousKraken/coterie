## 1. Create the module directory

- [x] 1.1 Create `src/service/member_service/` directory.
- [x] 1.2 Create empty `mod.rs`, `status.rs`, `dues.rs`, `updates.rs`, `create.rs`, `queries.rs`, `events.rs`.
- [x] 1.3 Delete the old `src/service/member_service.rs` only AFTER its contents have been moved (do this last, after all moves and a successful `cargo build`).

## 2. Move the struct and constructor

- [x] 2.1 Move `MemberService` struct definition (~lines 60-100 of the current file) to `mod.rs`.
- [x] 2.2 Move `pub fn new(...)` to `mod.rs`.
- [x] 2.3 Add `mod status; mod dues; mod updates; mod create; mod queries; mod events;` at the top of `mod.rs`.
- [x] 2.4 Add `pub use ...` if any items need to be re-exported (most likely the `MemberService` struct itself is already discoverable via `crate::service::member_service::MemberService` once the module exists).

## 3. Move status methods

- [x] 3.1 Move `activate`, `suspend`, `expire_now` to `status.rs` inside `impl MemberService { ... }`.
- [x] 3.2 Add the required `use` statements at the top of `status.rs` (look at the current `use` block in `member_service.rs` and bring only what `status.rs`'s methods actually use).
- [x] 3.3 Move the corresponding tests (`activate_emits_full_chain`, `activate_propagates_repo_error`, `suspend_emits_full_chain`, `expire_now_invalidates_sessions_and_audits`) into a `#[cfg(test)] mod tests { ... }` block at the bottom of `status.rs`.

## 4. Move dues methods

- [x] 4.1 Move `extend_dues`, `set_dues` to `dues.rs`.
- [x] 4.2 Imports.
- [x] 4.3 Move tests `extend_dues_validates_range`, `set_dues_writes_audit`.

## 5. Move updates methods

- [x] 5.1 Move `update`, `update_discord_id`, `resend_verification` to `updates.rs`.
- [x] 5.2 Imports.
- [x] 5.3 Move tests `update_emits_audit_and_event`, `update_discord_id_validates_snowflake`, `resend_verification_audits_on_success_and_rejects_verified`.

## 6. Move create methods

- [x] 6.1 Move `create`, `bulk_import`, `send_welcome_email` to `create.rs`. (`send_welcome_email` is private and called by both — keep it as `pub(super) async fn` or `async fn` depending on whether other submodules also call it.)
- [x] 6.2 Imports — `bulk_import` is the biggest function and probably brings the most imports.
- [x] 6.3 Move tests `create_audits_and_skips_activation_event` plus any bulk_import tests.
- [x] 6.4 If `create.rs` ends up >400 lines including tests, extract `bulk_import` into a `bulk_import.rs` sibling and update `mod.rs` to declare `mod bulk_import;`. Make `send_welcome_email` `pub(super)` in that case.

## 7. Move queries methods

- [x] 7.1 Move `audit_export`, `membership_type_name` to `queries.rs`.
- [x] 7.2 Imports.
- [x] 7.3 Move any tests covering these.

## 8. Move events helper

- [x] 8.1 Move `dispatch_member_updated` (private) to `events.rs`. Make it `pub(super)` so other submodules can call it.
- [x] 8.2 Update callers in `updates.rs` / `status.rs` to call via the appropriate path (`use super::events::dispatch_member_updated;` or `super::events::dispatch_member_updated(...)`).

## 9. Test helpers

- [x] 9.1 The current file's `#[cfg(test)]` block has `fresh_pool`, `make_service`, `make_member`, `audit_count` helpers shared across the tests. Place these in either:
   - (a) `src/service/member_service/test_helpers.rs` declared as `#[cfg(test)] mod test_helpers;` in `mod.rs`, OR
   - (b) A `#[cfg(test)] pub(super) mod test_helpers { ... }` directly inside `mod.rs`.
   Pick whichever's cleaner. Either way, sibling test modules access via `use super::test_helpers::*;` or `use crate::service::member_service::test_helpers::*;`.

## 10. Validation

- [x] 10.1 `cargo build --features test-utils` — clean compile, no warnings.
- [x] 10.2 `cargo test --features test-utils` — all tests pass. Compare test count to the pre-split baseline; should be identical.
- [x] 10.3 `cargo clippy --features test-utils -- --deny warnings` — clean.
- [x] 10.4 `cargo fmt --check` — clean.
- [x] 10.5 `wc -l src/service/member_service/*.rs` — confirm no file exceeds 400 lines.
- [x] 10.6 Delete the old `src/service/member_service.rs` file (since it's been replaced by the directory). Verify `cargo build` still succeeds after deletion.
