## 1. Service surface

- [x] 1.1 In `src/service/member_service.rs`, add the typed input/output structs:
  ```rust
  pub struct ImportRow { pub email: String, pub username: String, pub full_name: String,
                         pub membership_type_slug: String, pub status: Option<MemberStatus>,
                         pub notes: Option<String>, pub discord_id: Option<String> }
  pub struct ImportFailure { pub row_index: usize, pub email: Option<String>, pub reason: String }
  pub struct BulkImportSummary { pub succeeded: u32, pub failed: u32,
                                  pub failures: Vec<ImportFailure>,
                                  pub created_member_ids: Vec<Uuid> }
  ```
- [x] 1.2 Add `MemberService::bulk_import(actor_id: Uuid, rows: Vec<ImportRow>) -> Result<BulkImportSummary>`. Body:
  - For each row (with `row_index` starting at 1 for the first data row):
    1. Validate email format, non-empty username/full_name.
    2. Look up `membership_type_slug` via the membership-type service; failure = per-row failure.
    3. Build a `CreateMemberRequest`, call `member_repo.create(...)`. UNIQUE violation = per-row failure ("Email already exists" or "Username already exists" based on the SQL error).
    4. On success: audit-log `import_member` with the new member's id, push to `succeeded` count and `created_member_ids`.
    5. On any failure: push to `failed` count and `failures` vec, continue to the next row.
  - After all rows: audit-log `import_members_batch` with `entity_id = "*"`, `new_value = Some(format!("file={},succeeded={},failed={}", file_name, succeeded, failed))`.
  - Return the summary.
- [x] 1.3 The service does NOT do CSV parsing. It takes already-parsed `Vec<ImportRow>` and returns a typed summary.

## 2. Handler — GET form page

- [x] 2.1 Add `admin_members_import_page` handler in `src/web/portal/admin/members.rs`. Renders `templates/admin/member_import.html` (the upload form with a file input and a format reminder).
- [x] 2.2 Granular extractors: `State<Arc<CsrfService>>` (for the BaseContext), `Extension<CurrentUser>`, `Extension<SessionInfo>`.

## 3. Handler — POST import submit

- [x] 3.1 Add `admin_members_import` handler accepting `Multipart`. Granular extractors: `State<Arc<MemberService>>`, `Extension<CurrentUser>`, plus the multipart extractor.
- [x] 3.2 Body steps:
  - Read the multipart `file` field; cap at 5 MB; record the original filename.
  - Parse the file as CSV via `csv::ReaderBuilder::new().has_headers(true).from_reader(...)`.
  - Validate the header: required columns (`email`, `username`, `full_name`, `membership_type_slug`) must be present. If any missing, return an error fragment.
  - Iterate the CSV records into a `Vec<ImportRow>` (skip any with badly-typed fields, accumulating as failures — alternatively, fail at the service layer; pick the cleaner shape during implementation).
  - Call `member_service.bulk_import(current_user.id, rows).await`.
  - Render `templates/admin/member_import_result.html` with the summary (HTMX fragment).

## 4. Templates

- [x] 4.1 Create `templates/admin/member_import.html`. Includes:
  - `<form hx-post="/portal/admin/members/import" hx-target="#import-result" hx-encoding="multipart/form-data">`
  - `<input type="file" name="file" accept=".csv">`
  - `<input type="hidden" name="csrf_token" value="{{ base.csrf_token }}">`
  - A submit button.
  - A `<details>` block with the format reminder (required/optional columns, an example single-row CSV).
  - `<div id="import-result"></div>` for the HTMX response target.
- [x] 4.2 Create `templates/admin/member_import_result.html` (HTMX fragment). Includes:
  - Big-number summary: "N imported, M failed".
  - Conditional: if `failures.is_empty()`, just the success message.
  - Otherwise: a `<ul>` of failures, each showing row index + email (if any) + reason.
  - A link/button back to the members page.

## 5. Route registration

- [x] 5.1 In `src/web/portal/mod.rs`, add to the admin sub-router:
  - `.route("/members/import", get(admin::members::admin_members_import_page))`
  - `.route("/members/import", post(admin::members::admin_members_import))`
- [x] 5.2 The post route inherits the existing CSRF + admin gate. Verify multipart bodies don't trip CSRF (the existing security-middleware spec covers multipart CSRF — see `src/api/middleware/security.rs`).

## 6. UI link on members page

- [x] 6.1 In `templates/admin/members.html`, add a "Bulk import" link near the existing "New Member" button, pointing at `/portal/admin/members/import`.

## 7. Tests

- [x] 7.1 Add `tests/admin_member_import_test.rs` (new file). Boot the router with an in-memory pool, seed an admin and an active membership type with slug `regular`.
- [x] 7.2 Test: happy path. Build a small CSV in-memory with 3 valid rows; POST as multipart; assert response status 200; assert 3 `members` rows exist; assert 3 `import_member` audit rows + 1 `import_members_batch` audit row.
- [x] 7.3 Test: partial-failure. CSV has 2 valid rows + 1 row with a duplicate email; POST; assert 2 members created, response fragment lists the 1 failure with the duplicate email.
- [x] 7.4 Test: malformed CSV — missing `email` column in header. POST; assert response is the format-error message; no members created.
- [x] 7.5 Test: unknown `membership_type_slug` on a row. POST; assert that row is in failures with the unknown slug in the reason.

## 8. Validate

- [x] 8.1 `cargo build --all-targets --features test-utils` — clean.
- [x] 8.2 `cargo test --features test-utils` — full suite passes (one pre-existing unrelated date-flake in `weekly_creates_about_52_occurrences` confirmed to fail on `agent-q` without this change).
