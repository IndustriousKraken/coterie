## Why

`src/web/portal/admin/members.rs` houses two distinct concerns:

1. Single-member admin handlers — list page, detail page, create, activate, suspend, update, extend-dues, etc. The "primary" file purpose.
2. **Bulk CSV operations** — export (240-ish, plus `build_members_csv` helper at 315), and import (the import-page template, the result/error templates, the `ImportFailureView` view-model, the `admin_members_import_page` and `admin_members_import` handlers, and the `parse_import_csv` + `import_error_fragment` helpers — together about 250 lines starting at 1446).

These bulk-CSV pieces were added by `a12` and `a13` and are logically separate from the per-member admin actions. They share the file only because they share an admin gate. They each have their own templates, their own helpers, their own view-models. They're a coherent sub-module.

An architectural reviewer flagged that `members.rs` (1651 lines) has outgrown its identity. Lifting `admin_refund_payment` into a payments service is one half of the answer (`a18-lift-refund-payment-admin`); extracting these bulk handlers into a sibling sub-module is the other half. After both land, `members.rs` is back to a coherent ~1000-line file that exclusively serves "the admin members page and per-member actions."

## What Changes

- **Restructure `members.rs` into a `members/` directory module**:
  - `src/web/portal/admin/members.rs` becomes `src/web/portal/admin/members/mod.rs`.
  - The bulk-CSV pieces extract into a new `src/web/portal/admin/members/bulk.rs`.
- **`bulk.rs` contents**:
  - `admin_members_export` handler.
  - `build_members_csv` helper.
  - `AdminMemberImportPageTemplate`, `AdminMemberImportResultTemplate`, `AdminMemberImportErrorTemplate` template structs.
  - `ImportFailureView` view-model.
  - `admin_members_import_page` handler.
  - `admin_members_import` handler.
  - `parse_import_csv` helper.
  - `import_error_fragment` helper.
- **`members/mod.rs`** re-exports from `bulk` so the public names stay valid — route registration in `src/web/portal/mod.rs` continues to read `admin::members::admin_members_export`, etc., unchanged. Internal organization becomes an implementation detail of the `members` module.
- **Out of scope**:
  - Any behavior change. The handlers, helpers, templates, view-models all stay identical — just relocated.
  - Touching the route table.
  - Moving `MemberExportRow` (which lives in `member_repository.rs`) or `ImportRow`/`ImportFailure`/`BulkImportSummary` (in `member_service.rs`). Those are domain types, correctly located on the service/repo side.
  - Renaming any function or type.

## Capabilities

### New Capabilities

(None — pure file reorganization.)

### Modified Capabilities

(None — no behavioral or API-shape change. The internal module organization is a code-style concern, not a spec concern. If desired, a small `admin-members` spec note can document that bulk CSV operations live in a sub-module, but that's a code-organization detail; specs typically don't constrain file layout below the capability boundary.)

## Impact

- **Code**:
  - `src/web/portal/admin/members.rs` (1651 lines) is renamed to `src/web/portal/admin/members/mod.rs` and shrinks by ~400 lines (the bulk pieces leave).
  - New `src/web/portal/admin/members/bulk.rs` (~400 lines).
  - `pub mod bulk;` + `pub use bulk::*;` added to `members/mod.rs`.
- **Wire shape**: zero change. Same URLs, same templates rendered, same audit row contents, same CSV byte output.
- **Route file**: `src/web/portal/mod.rs` is unchanged — the `pub use bulk::*;` in `members/mod.rs` keeps the public names addressable as `admin::members::admin_members_export` etc.
- **Tests**: existing tests pass unchanged.
- **Risk**: very low. Pure file moves with re-export plumbing.
- **Dependency**: none. Independent of `a17` (EmailTokenService) and `a18` (refund lift); all three can run in any order.
