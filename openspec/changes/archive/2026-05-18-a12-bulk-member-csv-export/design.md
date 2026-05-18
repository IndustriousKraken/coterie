## Context

The audit-log export at `src/web/portal/admin/audit.rs::audit_log_export` is the model: a `push_csv(&mut out, value)` helper that handles quoting, plus a `Vec<u8>` accumulator, plus a `Response::builder()` setting `Content-Type` and `Content-Disposition`. ~50 lines of focused code. Mirror that.

## Goals / Non-Goals

**Goals:**
- Single-click roster export from `/portal/admin/members`.
- Filter state preserved — the export reflects whatever filters the admin has applied.
- All non-credential fields included.
- Audit-logged so abuse is traceable.

**Non-Goals:**
- Excel `.xlsx` format. CSV is universal; consumers can open in Excel.
- Streaming. Even with 10k members the export is well under 10 MB; buffer in memory.
- Per-column toggles. Export everything that's not a credential.
- Including the password hash, TOTP secret, recovery codes, or any other credential field.
- Including the Stripe customer/subscription IDs. Out of scope to leak Stripe identifiers via export.

## Decisions

### D1. Reuse the existing `push_csv` helper or extract a shared one

The audit export defines `push_csv` privately in `audit.rs`. The cleanest move is to lift it to a shared helper module — `src/web/portal/admin/csv.rs` — and have both audit and member exports call it. Alternatively, copy the function inline. Copy-then-extract-later is fine for v1 if the lift adds friction; choose based on what's easier in the implementing context.

### D2. Reuse `MemberQuery` for filter inputs

The admin members page already builds a `MemberQuery` from the URL query string. The export handler does the same — parses the query string into `MemberQuery`, runs it through the repo. The export ignores pagination (`limit`, `offset`) — it wants all matches.

### D3. Repository method shape

Option A: `MemberRepository::search_all(MemberQuery) -> Result<Vec<(Member, String)>>` (member + membership type name). Cleanest type.

Option B: reuse `MemberRepository::search(MemberQuery)` with `limit = i64::MAX`. Risks performance on a huge roster but simpler.

Option C: build a dedicated `MemberRepository::export_rows(filter)` that returns a flat `MemberExportRow` struct with the joined fields the CSV needs. Cleanest output type for the handler.

Pick **C**. The `MemberExportRow` struct is a thin DTO containing exactly the CSV columns. Repo method is one SQL query (JOIN against `membership_types`). Handler doesn't have to do per-member lookups for the type name.

### D4. CSV column order is stable

The first row of the file is the header (column names). The order matches the proposal exactly. Stability matters — operators may write scripts against the export.

### D5. Audit emission

The handler emits the audit row via `MemberService::export_members(actor_id, query_summary, row_count)` — a new thin service method. The reason for putting this on the service: every member-touching action (read or write) that's admin-initiated and traceable goes through `MemberService`. The audit row's `entity_id` is `*` (wildcard); the `new_value` is the row count plus a brief summary of the filter (e.g., `"status=Active,count=42"`).

Alternative: emit the audit row directly in the handler. Simpler, but breaks the pattern that lift-member-admin-orchestration established. Service-locus wins.

### D6. Filename includes the date

`members-export-YYYY-MM-DD.csv` (UTC date of the export time). Operators downloading multiple times in a day get the same name with the same date suffix — they'll deal. Adding a timestamp would change the name on every click; the date is enough.

### D7. Content-Disposition: attachment

The response sets `Content-Disposition: attachment; filename="..."` so the browser downloads rather than rendering. The MIME type is `text/csv; charset=utf-8`.

## Risks / Trade-offs

- **Risk**: large export (10k+ members) holds a multi-megabyte response in memory. → Trade-off: acceptable at the codebase's scale. If a single-tenant Coterie instance grows to 100k members, this might warrant a streaming rewrite, but it'd be the smallest of many problems at that point.
- **Risk**: export leaks PII via a forwarded URL with a session cookie. → Mitigation: the `require_admin_redirect` gate ensures only admins reach the route. Audit row catches abuse.
- **Trade-off**: include `notes` in the export. Free-form admin notes can contain anything. Decision: include them — the export is admin-only and the admin who wrote the notes is downloading them. A future change could add a `?include_notes=false` toggle for narrower exports.

## Migration Plan

Single PR.

1. Lift `push_csv` to a shared helper (or copy inline — decide during implementation).
2. Add `MemberExportRow` struct in `src/repository/member_repository.rs` (or wherever `MemberQuery` lives, post-`a04`).
3. Add `MemberRepository::export_rows(filter: MemberQuery) -> Result<Vec<MemberExportRow>>` to the trait + impl. Query joins against `membership_types`.
4. Add `MemberService::export_members(actor_id, filter, row_count) -> Result<()>` that just audit-logs (doesn't fetch). Or alternative: the handler calls `member_service.audit_members_export(...)` after building the response.
5. Add `admin_members_export` handler in `src/web/portal/admin/members.rs`. Wires the parsed query → repo call → CSV string → response.
6. Register the route in `src/web/portal/mod.rs`: `.route("/members/export", get(admin::members::admin_members_export))` under the admin sub-router.
7. Add a "Download CSV" link on the admin members page (`templates/admin/members.html`) that preserves the current filter query string.
8. Integration test: seed members, GET the export endpoint, parse the CSV, assert column order + row count.
9. Integration test: same endpoint with `?status=Active`, assert only Active members.
10. `cargo test --features test-utils` — green.
