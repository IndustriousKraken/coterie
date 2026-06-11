## Why

Admins occasionally need to pull the member roster out of Coterie — for an annual report, a backup, or a one-off integration. Today there's no export path; an admin would have to query the SQLite database directly. The TODO has "Bulk member import/export" as an open item.

The audit-log admin page already implements CSV export (`src/web/portal/admin/audit.rs::audit_log_export`) with a hand-rolled CSV writer that escapes commas, quotes, and newlines correctly. This change mirrors that for members.

## What Changes

- **New admin route**: `GET /portal/admin/members/export` returning `text/csv`. The download filename is `members-export-YYYY-MM-DD.csv`.
- **CSV columns** (one row per member, including all statuses):
  - `id` (UUID)
  - `email`
  - `username`
  - `full_name`
  - `status` (Active / Pending / Expired / Suspended / Honorary)
  - `membership_type` (display name — joined from `membership_types.name`)
  - `joined_at` (ISO 8601 with timezone)
  - `dues_paid_until` (ISO 8601 or empty)
  - `is_admin` (true/false)
  - `bypass_dues` (true/false)
  - `discord_id` (snowflake or empty)
  - `email_verified_at` (ISO 8601 or empty)
  - `notes` (free-form admin notes — escaped)
- **Filter support**: respects the same query string as the admin members page (`?q=`, `?status=`, `?type=`). The export reflects whatever the current filtered view is.
- **Admin-only**, behind the existing `require_admin_redirect` gate. The export contains PII (emails, real names, admin notes) — only admins.
- **Audit row**: writes `audit_logs (action='export_members', entity_type='member', entity_id='*', new_value=Some(row_count))`. So an admin sweeping the roster is traceable.
- **No new dependency**: hand-rolled CSV writer follows the `push_csv` helper pattern from `audit_log_export`.
- **Out of scope**: scheduling exports (manual click-to-download is fine for now); per-column selection (export everything that's not a credential); CSV import (separate change — `a13`).

## Capabilities

### New Capabilities
- `bulk-member-csv-export`: admins can download the member roster as CSV via a single click; export respects current filter state.

### Modified Capabilities
- `admin-members`: gains the export route.
- `audit-logging`: adds the `export_members` action to the documented action vocabulary.

## Impact

- **Code**:
  - New handler in `src/web/portal/admin/members.rs::admin_members_export`. Reuses the existing `MemberQuery` typed-filter shape from `repository/member_repository.rs`.
  - New repository method `MemberRepository::search_all(query: MemberQuery) -> Result<Vec<MemberExportRow>>` — returns the joined member + membership-type-name shape without pagination (export wants everything that matches the filter).
  - Or: reuse the existing `MemberRepository::search` with `limit = i64::MAX` (or a generous cap like 100k). Decision in design.
  - Route registered in `src/web/portal/mod.rs` under the admin sub-router.
  - Audit-log emission in the handler (or in a new `MemberService::export_members` method that does the audit — design decides).
- **Wire shape**: one new GET route. Response is `Content-Type: text/csv` with a `Content-Disposition: attachment; filename=...` header.
- **Tests**: integration test boots a router with seeded members in various states, GETs the export endpoint with a session cookie + admin permissions, asserts the response body parses as CSV with the expected columns and row count. Filter test: same setup with `?status=Active`, asserts only Active rows in the output.
- **Risk**: low. Bounded, well-known pattern.
- **Dependency**: none beyond what's already in main. The `lift-member-admin-orchestration` change has landed, so the audit emission can go through `MemberService` if the design favors that.
