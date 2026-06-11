## Context

`members.rs` at 1651 lines mixes two distinct concerns. The bulk-CSV pieces (export + import) form a coherent ~400-line sub-unit: they have their own templates, view-models, helpers, and they share no state with the per-member handlers beyond going through `MemberService`. Extracting them into a sibling module is mechanical.

The Rust idiom for converting `foo.rs` into a directory module is:
- Rename `foo.rs` to `foo/mod.rs`.
- Add sibling files `foo/bar.rs`, `foo/baz.rs` for sub-modules.
- In `mod.rs`, declare `pub mod bar;` and optionally `pub use bar::*;` to keep the public surface flat.

Route registration in `src/web/portal/mod.rs` reads function paths like `admin::members::admin_members_export`. With `pub use bulk::*;` in `members/mod.rs`, that path keeps resolving — the route file doesn't need to know that `admin_members_export` now physically lives in `members/bulk.rs`.

## Goals / Non-Goals

**Goals:**
- Bulk-CSV handlers and their support code live in `src/web/portal/admin/members/bulk.rs`.
- `src/web/portal/admin/members/mod.rs` contains only the per-member handlers and supporting code.
- Public names (handler function names, the types route registration uses) stay reachable at their current paths.
- Wire shape and behavior unchanged.

**Non-Goals:**
- Splitting the per-member handlers further (the remaining file is coherent at ~1000 lines).
- Renaming functions, types, or templates.
- Moving domain types (`MemberExportRow`, `ImportRow`, etc.) from their current homes on the repo/service side.
- Touching route registration in `src/web/portal/mod.rs`.

## Decisions

### D1. Directory module, not sibling file

Considered: leave `members.rs` in place and create a sibling `members_bulk.rs`. Rejected — the underscore-naming convention is uncommon in this codebase, and "sibling" doesn't communicate the parent-child relationship as cleanly as the directory pattern.

The directory pattern (`members/mod.rs` + `members/bulk.rs`) is idiomatic Rust and matches what `src/web/portal/admin/` already does for the admin sub-tree.

### D2. `pub use bulk::*;` in `members/mod.rs`

Keep the route file unchanged. Re-exporting the public names from `bulk` flattens the path so `admin::members::admin_members_export` continues to resolve.

Alternative: update the route file to use `admin::members::bulk::admin_members_export`. Slightly more explicit but adds churn in `src/web/portal/mod.rs` that isn't needed. The re-export pattern is established elsewhere in the codebase (e.g., `crate::repository::MemberQuery` re-exports from `crate::repository::member_repository`).

### D3. Move both export AND import together

Even though export was added by `a12` and import by `a13`, they're the same conceptual feature ("bulk CSV operations on the member roster"). Moving them together gives one file a coherent purpose. Splitting them across two files would re-create the same "mixed concerns" problem at a smaller scale.

### D4. The CSV-writer helper (`build_members_csv`) goes with the export handler

`build_members_csv` is private and only called by `admin_members_export`. Move it.

### D5. The CSV-reader helper (`parse_import_csv`) goes with the import handler

Same shape. Private; only called by `admin_members_import`. Move it.

### D6. View-model `ImportFailureView` goes to `bulk.rs`

It's only used by the import-result template, which moves with the import handler. Move the view-model too.

### D7. Imports inside `bulk.rs` reference parent module via `super::*` or explicit paths

The bulk handlers reference shared types from the parent (e.g., `AdminMembersQuery` for the export filter, possibly some shared template-context utilities). Use `super::` for cross-module references within the same module tree:

```rust
// inside bulk.rs
use super::AdminMembersQuery;  // if the type lives in members/mod.rs
```

If the existing types are imported from outside the module (e.g., `crate::repository::MemberQuery`), those imports stay the same — they're already absolute paths.

## Risks / Trade-offs

- **Risk**: a `use` statement gets missed during the move and `bulk.rs` won't compile. → Mitigation: compiler catches it; fix imports until clean.
- **Risk**: the `pub use bulk::*;` re-export collides with a name in `members/mod.rs`. → Verify during implementation; if any name collision exists, rename one or use explicit re-exports rather than the glob.
- **Trade-off**: introduces a tiny indirection — readers of `members/mod.rs` need to know to look in `bulk.rs` for the export/import handlers. This is the entire point of the change; the indirection is a feature.

## Migration Plan

Single PR.

1. Create the directory: `mkdir src/web/portal/admin/members && mv src/web/portal/admin/members.rs src/web/portal/admin/members/mod.rs`.
2. `cargo build` — clean (nothing references the old path; the file is just relocated).
3. Identify all bulk-related items in `members/mod.rs` (the list in the proposal's What Changes section).
4. Create `src/web/portal/admin/members/bulk.rs`. Move each item from `members/mod.rs` to `bulk.rs` one at a time, fixing imports as the compiler complains. Each item is a self-contained move.
5. In `members/mod.rs`, add `pub mod bulk;` and `pub use bulk::*;` near the top of the file.
6. `cargo build --all-targets --features test-utils` — clean.
7. `cargo test --features test-utils` — full suite passes.
8. Eyeball: `wc -l src/web/portal/admin/members/{mod.rs,bulk.rs}` — `mod.rs` should be ~1200 lines (after `a18` lift, ~1050; after this extract, ~800–900); `bulk.rs` should be ~400 lines.
