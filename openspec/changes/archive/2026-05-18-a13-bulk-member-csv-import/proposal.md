## Why

The companion to `a12-bulk-member-csv-export`. Onboarding a new Coterie instance often means importing an existing member list from a previous system (Excel sheet, an old CMS, hand-maintained Google Doc). Today the only way is to hand-create each member through the admin UI, one at a time. The TODO has "Bulk member import/export" as an open item.

The import scope is the inverse of the export: the same columns, the same CSV format, the same admin-only access. The import is more delicate than the export because:

1. Rows can fail validation (bad email, duplicate username, unknown membership type).
2. Partial-success semantics matter — operators want to know which rows failed and why.
3. Credentials need special handling — imported members have no password; they'll have to use password-reset to claim their account.

## What Changes

- **New admin route**: `POST /portal/admin/members/import` accepting `multipart/form-data` with a CSV file upload. Returns an HTML fragment summarizing successes and per-row failures.
- **CSV columns** (subset of the export, plus a marker for membership-type lookup):
  - **Required**: `email`, `username`, `full_name`, `membership_type_slug` (looked up against `membership_types.slug`).
  - **Optional**: `status` (defaults to Pending), `notes`, `discord_id`. Other fields (`id`, `joined_at`, `is_admin`, `bypass_dues`, `email_verified_at`, `dues_paid_until`) are NOT importable in v1 — they're either auto-assigned or admin-only-via-UI.
- **Service method**: `MemberService::bulk_import(actor_id, rows: Vec<ImportRow>) -> Result<BulkImportSummary>`. Returns a summary with per-row success/failure plus aggregate counts. Each successful row creates a member with status = Pending (or the row's specified value), audit-logs `import_member` per row.
- **Failure modes** that should be per-row (not abort-the-whole-batch):
  - Invalid email format
  - Duplicate email or username
  - Unknown `membership_type_slug`
  - Invalid `status` value
- **Failure modes** that abort the batch:
  - CSV parse error (missing required columns, malformed file)
  - File too large (cap: 5 MB)
- **Audit-log entry per imported member** plus one aggregate entry `import_members_batch` with the success/failure counts.
- **Out of scope**: setting passwords (imported members must use password-reset); sending welcome emails on import (operator chooses when to "go live" by activating); updates to existing members (this is INSERT-only; matching email or username is a failure, not an upsert).
- **The UI**: a "Bulk import" link on the admin members page, opening a form with file upload, file format reminder, and submit. The result fragment is rendered inline below the form.

## Capabilities

### New Capabilities
- `bulk-member-csv-import`: admins can upload a CSV file to create multiple members in one operation. Per-row error reporting.

### Modified Capabilities
- `admin-members`: gains the import route and UI.
- `audit-logging`: adds `import_member` (per-row) and `import_members_batch` (aggregate) to the documented action vocabulary.

## Impact

- **Code**:
  - New handler in `src/web/portal/admin/members.rs::admin_members_import` accepting multipart.
  - New `ImportRow` and `BulkImportSummary` types in `src/service/member_service.rs` (or alongside it).
  - New `MemberService::bulk_import` method that validates each row, calls `MemberRepository::create` for each valid one, collects per-row outcomes.
  - New template `templates/admin/member_import.html` (form) and `templates/admin/member_import_result.html` (HTMX result fragment).
  - Route registered in `src/web/portal/mod.rs` under the admin sub-router.
- **Wire shape**: one new POST endpoint accepting multipart. The response is HTML (matches the rest of the admin UI's HTMX patterns).
- **Tests**: integration tests covering a successful import, a partial-failure import (some valid + some duplicate), an abort-the-batch case (malformed CSV).
- **Risk**: medium. Member creation is a real side-effect; bulk operations make mistakes amplify. Mitigation: the import handler explicitly never updates existing rows (duplicate detected → per-row failure, no overwrite); the audit trail covers what was created.
- **Dependency**: depends on `lift-member-admin-orchestration` having shipped (which it has — `MemberService` is the right home for `bulk_import`). Also independent of `a12-bulk-member-csv-export`; both can run in either order, though for queue clarity `a13` follows `a12`.
