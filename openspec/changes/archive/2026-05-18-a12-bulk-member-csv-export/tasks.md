## 1. Repository surface

- [x] 1.1 In `src/repository/member_repository.rs` (or wherever `MemberQuery` lives, post-`a04`), add `pub struct MemberExportRow { id: Uuid, email: String, username: String, full_name: String, status: MemberStatus, membership_type: String, joined_at: DateTime<Utc>, dues_paid_until: Option<DateTime<Utc>>, is_admin: bool, bypass_dues: bool, discord_id: Option<String>, email_verified_at: Option<DateTime<Utc>>, notes: Option<String> }`.
- [x] 1.2 Add `MemberRepository::export_rows(filter: MemberQuery) -> Result<Vec<MemberExportRow>>` to the trait. The implementation joins `members` against `membership_types` for the name; reuses the filter-clause logic from the existing `search` method but ignores pagination.

## 2. Service-layer audit

- [x] 2.1 Add `MemberService::audit_export(actor_id: Uuid, filter_summary: &str, row_count: usize) -> Result<()>` that writes the audit row with `action = "export_members"`, `entity_type = "member"`, `entity_id = "*"`, `new_value = Some(filter_summary + ",count=" + row_count)`. The method ONLY does the audit — it doesn't fetch data. Naming alternative: `record_export` if `audit_export` reads ambiguously.

## 3. CSV writer

- [x] 3.1 Decide: lift `push_csv` from `src/web/portal/admin/audit.rs` to a shared helper in `src/web/portal/admin/csv.rs` (preferred), OR copy inline. Either way: the same RFC 4180 escaping the audit export already implements.

## 4. Handler

- [x] 4.1 Add `admin_members_export` handler in `src/web/portal/admin/members.rs`. Granular extractors: `State<Arc<dyn MemberRepository>>`, `State<Arc<MemberService>>`, `Extension<CurrentUser>`, `Query<AdminMembersQuery>` (the existing query-string type the index page already uses).
- [x] 4.2 Body: parse the query into a `MemberQuery` (reuse the same parsing the index page does), call `export_rows`, build a CSV string with the header + one row per `MemberExportRow`, call `member_service.audit_export(current_user.id, filter_summary, rows.len())`, return a `Response` with `Content-Type: text/csv; charset=utf-8` and `Content-Disposition: attachment; filename="members-export-{YYYY-MM-DD}.csv"`.
- [x] 4.3 Build the date suffix from `Utc::now().date_naive().format("%Y-%m-%d")`.
- [x] 4.4 Build the `filter_summary` string from the original query string (e.g., `q=ab&status=Active` → `"q=ab,status=Active"`). Empty filter → empty string before the `count=N` suffix.

## 5. Route registration and UI link

- [x] 5.1 In `src/web/portal/mod.rs`, add the route under the admin sub-router: `.route("/members/export", get(admin::members::admin_members_export))`. Keep it inside the `require_admin_redirect`-gated tier.
- [x] 5.2 In `templates/admin/members.html` (the page that lists members), add a "Download CSV" link or button near the filter controls. The link's `href` is `/portal/admin/members/export?<current-query-string>`. If Askama makes the query-string preservation awkward, fall back to a small `<form action="/portal/admin/members/export" method="get">` containing hidden inputs for the active filters.

## 6. Tests

- [x] 6.1 Add `tests/admin_member_export_test.rs` (new file). Boot the router with an in-memory pool. Seed 3 members with different statuses, a non-default membership-type. Authenticate as admin via the existing test session helper.
- [x] 6.2 Test: GET `/portal/admin/members/export` returns 200, `Content-Type: text/csv; charset=utf-8`. The body parses as CSV; header row matches the spec's column order; row count is 3.
- [x] 6.3 Test: GET `/portal/admin/members/export?status=Active` returns only the Active member(s).
- [x] 6.4 Test: seed a member with `full_name = "O'Brien, Sean"` and `notes = "Has \"complications\""`. Export and parse. Assert the special characters are correctly escaped per RFC 4180.
- [x] 6.5 Test: after a successful export, an `audit_logs` row with `action = "export_members"` exists for the test admin's `actor_id`.

## 7. Validate

- [x] 7.1 `cargo build --all-targets --features test-utils` — clean.
- [x] 7.2 `cargo test --features test-utils` — full suite passes. (Pre-existing date-flaky failure in `recurring_event_test::weekly_creates_about_52_occurrences` is unrelated — anchor is 2026-05-05 and the test ran 2026-05-18.)
