## Context

CSV import is a common bootstrapping need. The shape — admin uploads a file, system validates row-by-row, reports successes and failures — is well-established in admin tooling everywhere.

Coterie's existing `MemberService` is the right home for the import logic. The CLAUDE.md rule says side-effects (audit log, integration events, emails) belong in the service; bulk creation is no exception — the service emits per-row audit rows plus an aggregate batch summary row.

## Goals / Non-Goals

**Goals:**
- A single admin click imports a CSV of members.
- Per-row errors don't abort the batch.
- The operator sees exactly which rows succeeded and which failed, with reasons.
- No password is set on import; imported members must use password-reset.
- Imported members start as Pending unless the CSV specifies otherwise.

**Non-Goals:**
- Update-or-insert semantics. v1 is INSERT-only; duplicate detection is a per-row failure.
- Setting passwords from the CSV. Treat any `password` column as ignored (audit-log a warning if present, but don't error).
- Sending welcome emails on import. The operator activates members later as part of their go-live plan; activation triggers the welcome email per the existing `MemberService::activate` semantics.
- Importing roles/admin flags. v1 is members-only; admin promotion is a separate per-row UI action.
- Importing `id` / `joined_at` / `dues_paid_until`. The system controls these. Future versions might support `joined_at` for backfill scenarios; v1 keeps it simple.

## Decisions

### D1. Required vs. optional columns

```
Required: email, username, full_name, membership_type_slug
Optional: status (default "Pending"), notes (default empty), discord_id (default null)
```

The CSV header MUST contain at least the required columns. Extra columns are tolerated and silently ignored (the parser warns but doesn't fail — operators often paste a full export and edit it).

### D2. Per-row validation rules

Each row passes validation iff:

- `email` is non-empty and contains `@`.
- `username` is non-empty.
- `full_name` is non-empty.
- `membership_type_slug` exists and is `is_active` in `membership_types`.
- `status` (if present) parses as a valid `MemberStatus` variant.
- No existing member has the same `email` or `username`.

Failures are accumulated into a `Vec<ImportFailure>` and returned in the summary; the row is skipped.

### D3. Batch processing semantics

Process row-by-row in CSV order. Each row's success or failure is independent — one bad row doesn't poison subsequent rows. The summary returns the row count of each kind plus the full list of failures (with row index, the row's email/username for identification, and the failure reason).

A transactional bulk-insert (where any failure rolls back the whole batch) was considered and rejected. Operators want partial success ("9 out of 10 succeeded, here's the one that didn't") more than they want atomic safety ("none of them imported because row 3 had a typo"). The audit log makes the partial-success scenario fully traceable.

### D4. Audit-log emissions

Per-row success: `audit_logs (action='import_member', entity_type='member', entity_id=<new uuid>, new_value=Some(email))`. Same actor_id as the importing admin.

Aggregate at end: `audit_logs (action='import_members_batch', entity_type='member', entity_id='*', new_value=Some(summary_string))` where `summary_string` is e.g., `"file=members.csv,succeeded=42,failed=3"`.

### D5. Response shape

The handler returns an HTML fragment (HTMX-compatible) summarizing the result. The fragment includes:

- Big-number summary: "42 members imported, 3 failed."
- Per-failure list with row index, email (if parseable), and failure reason.
- A link back to the admin members page.

For HTMX-driven UX: the form's `hx-post` swaps the result into a target div on the same page. No full-page reload.

### D6. File size cap

5 MB. Enforced at the multipart parser before the handler reads. A 5 MB CSV holds ~50k rows at typical column widths — vastly more than any small-org Coterie instance will ever have, but enough headroom to avoid bumping the cap for a long time.

### D7. The handler is `MemberService::bulk_import`'s caller, not its co-owner

The handler does multipart parsing and result rendering. `MemberService::bulk_import` takes a typed `Vec<ImportRow>` and returns a `BulkImportSummary`. The handler parses CSV → rows → service → summary → render. The service doesn't know about CSV or HTML — keeping the layering clean.

```rust
pub struct ImportRow {
    pub email: String,
    pub username: String,
    pub full_name: String,
    pub membership_type_slug: String,
    pub status: Option<MemberStatus>,
    pub notes: Option<String>,
    pub discord_id: Option<String>,
}

pub struct ImportFailure {
    pub row_index: usize,
    pub email: Option<String>,
    pub reason: String,
}

pub struct BulkImportSummary {
    pub succeeded: u32,
    pub failed: u32,
    pub failures: Vec<ImportFailure>,
    pub created_member_ids: Vec<Uuid>,
}
```

### D8. CSV parsing

Use the `csv` crate (already a transitive dep via `sqlx` or similar — check; if not, add it as a direct dep). Parser: `csv::ReaderBuilder::new().has_headers(true).from_reader(...)`.

Header validation: the parser reads the header row first; if any required column is missing, return a batch-level error before processing any data rows.

### D9. UI access pattern

The admin members page gains a "Bulk import" link/button (next to the existing "New Member" and "Download CSV" buttons). Clicking opens `/portal/admin/members/import` (GET) which renders a simple form with the file input and a "Format reminder" snippet listing the required columns.

Form submission POSTs to `/portal/admin/members/import` with multipart. The HTMX response replaces the form area with the result fragment.

## Risks / Trade-offs

- **Risk**: an operator imports a sheet with auto-generated emails that don't actually receive mail. → Mitigation: members start Pending; until they're activated, they can't log in. Future password-reset triggers an email, which the operator can verify works before activating.
- **Risk**: duplicate detection misses a case (e.g., a member with a different-case email). → Mitigation: the existing `MemberRepository::create` uses the same UNIQUE constraints that handle this; the import inherits whatever logic the manual create has. If case-insensitive uniqueness isn't enforced today, the import inherits that gap (separate concern).
- **Trade-off**: no progress streaming. A 1000-row import will block the request for however long it takes to insert 1000 rows (probably seconds). Acceptable for v1.

## Migration Plan

Single PR.

1. Add `ImportRow`, `ImportFailure`, `BulkImportSummary` types to `src/service/member_service.rs`.
2. Add `MemberService::bulk_import(actor_id, rows: Vec<ImportRow>) -> Result<BulkImportSummary>`. Iterate, validate, create, audit per-row + batch.
3. Add the CSV parsing in the handler (NOT in the service — the service stays csv-agnostic).
4. Add the `admin_members_import` handler in `src/web/portal/admin/members.rs`. Multipart parsing, CSV parsing, service call, result rendering.
5. Add templates `templates/admin/member_import.html` (form page) and `templates/admin/member_import_result.html` (result fragment).
6. Register both routes in `src/web/portal/mod.rs` (GET form page, POST import submit).
7. Add "Bulk import" link on `templates/admin/members.html`.
8. Tests: happy-path import of N members; partial-failure import (some duplicates, some missing required columns); aborted import (missing header columns).
9. `cargo test --features test-utils` — green.
