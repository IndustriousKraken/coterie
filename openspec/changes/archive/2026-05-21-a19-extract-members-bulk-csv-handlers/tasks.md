## 1. Convert members.rs into a directory module

- [x] 1.1 `mkdir /Users/rab/Dropbox/code/coterie/src/web/portal/admin/members && git mv /Users/rab/Dropbox/code/coterie/src/web/portal/admin/members.rs /Users/rab/Dropbox/code/coterie/src/web/portal/admin/members/mod.rs` (use `git mv` so history is preserved).
- [x] 1.2 `cargo build` — clean. Nothing references the old file path; the file is just relocated.

## 2. Create bulk.rs and move the bulk-CSV pieces

- [x] 2.1 Create `src/web/portal/admin/members/bulk.rs` (empty).
- [x] 2.2 Move from `members/mod.rs` to `members/bulk.rs`, in order (each move is a cut from `mod.rs` and paste into `bulk.rs`):
  - `pub async fn admin_members_export(...)` and its full body.
  - `fn build_members_csv(...)` helper.
  - `pub struct AdminMemberImportPageTemplate`.
  - `pub async fn admin_members_import_page(...)`.
  - `pub struct AdminMemberImportResultTemplate`.
  - `pub struct ImportFailureView`.
  - `pub struct AdminMemberImportErrorTemplate`.
  - `pub async fn admin_members_import(...)` and its full body.
  - `fn parse_import_csv(...)` helper.
  - `fn import_error_fragment(...)` helper.
- [x] 2.3 In `members/mod.rs`, add `pub mod bulk;` and `pub use bulk::*;` near the top of the file (under the existing module-level `use` statements but before any item definitions).

## 3. Fix imports inside bulk.rs

- [x] 3.1 `cargo build`. The compiler will flag every missing `use` statement inside `bulk.rs` (types from `axum`, `chrono`, `crate::*`, `super::*`, the `MemberService`, the `csv` crate, etc.). Add the imports until clean.
- [x] 3.2 For any type that previously came from inside `members.rs` (e.g., `AdminMembersQuery`, possibly some shared template-base utility) that bulk.rs now needs, import via `use super::AdminMembersQuery;` or the analogous path.

## 4. Resolve any name collisions

- [x] 4.1 The `pub use bulk::*;` is a glob re-export. If any name in `bulk` collides with a name already in `members/mod.rs` (none expected, but possible if there's an internal helper named the same as a bulk helper), Rust will refuse to compile with a clear error. Resolve by either renaming one of the colliders or by replacing the glob with explicit re-exports: `pub use bulk::{admin_members_export, admin_members_import_page, admin_members_import};` (note: re-exporting only the route-referenced functions; the templates/view-models stay module-private — they're only used by the handlers in `bulk.rs` itself).
- [x] 4.2 Prefer the explicit re-export form over the glob if the file ends up with many internal-only items in `bulk.rs` that don't need to be addressable from outside.

## 5. Confirm route registration unchanged

- [x] 5.1 `grep -n "admin_members_export\|admin_members_import\|admin_members_import_page" /Users/rab/Dropbox/code/coterie/src/web/portal/mod.rs` — confirm the route registrations still use `admin::members::<name>`, not `admin::members::bulk::<name>`.
- [x] 5.2 If for some reason the explicit re-export form was needed in 4.1, ensure the three handler names are listed there.

## 6. Validate

- [x] 6.1 `cargo build --all-targets --features test-utils` — clean.
- [x] 6.2 `cargo test --features test-utils` — full suite passes. Existing integration tests for the export and import routes continue to pass without modification (their HTTP paths haven't changed).
- [x] 6.3 Eyeball: `wc -l /Users/rab/Dropbox/code/coterie/src/web/portal/admin/members/mod.rs /Users/rab/Dropbox/code/coterie/src/web/portal/admin/members/bulk.rs`. After this change (and assuming `a18` has also landed): `mod.rs` should be ~800–900 lines, `bulk.rs` ~400 lines. (If `a18` hasn't landed yet, `mod.rs` will be ~1000 lines instead.)
