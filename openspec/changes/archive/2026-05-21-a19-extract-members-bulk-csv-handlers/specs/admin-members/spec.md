## ADDED Requirements

### Requirement: Bulk-CSV admin handlers live in a sibling sub-module

The bulk-CSV admin operations (`admin_members_export`, `admin_members_import_page`, `admin_members_import`, plus their supporting templates and parse/render helpers) SHALL live in `src/web/portal/admin/members/bulk.rs`, a sub-module of the `members` admin module. The per-member admin handlers (single-member CRUD, status transitions, dues operations) SHALL live in `src/web/portal/admin/members/mod.rs`.

`members/mod.rs` SHALL re-export the public surface from `bulk` (typically via `pub use bulk::*;`) so route registration continues to resolve handler names at `admin::members::<name>` without needing to know the internal `bulk` sub-path.

The intent: `members/mod.rs` is the per-member admin page; `bulk.rs` is the roster-level bulk operations. Each file has a coherent identity. The shared parent module groups them under one URL family.

#### Scenario: New bulk-CSV handler lands in bulk.rs

- **WHEN** a contributor adds a new bulk-CSV admin operation (e.g., bulk export of payment history)
- **THEN** the handler, its template, and its helpers SHALL be added to `bulk.rs`, not to `mod.rs`

#### Scenario: New per-member handler lands in mod.rs

- **WHEN** a contributor adds a new per-member admin action (e.g., a "force-verify email" button)
- **THEN** the handler SHALL be added to `mod.rs`, not to `bulk.rs`

#### Scenario: Route registration stays flat

- **WHEN** the router file (`src/web/portal/mod.rs`) registers a bulk-CSV route
- **THEN** the handler path SHALL read `admin::members::admin_members_export` (or equivalent), NOT `admin::members::bulk::admin_members_export`; the `pub use bulk::*;` re-export flattens the path
