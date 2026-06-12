## Why

Two admin-facing render paths truncate free-text with **byte-indexed slicing**, which panics when the cut byte lands in the middle of a multi-byte UTF-8 character. The codebase already documents this exact bug class and a safe workaround at `src/integrations/discord.rs:49` ("The naive `&s[..280]` would panic on content with a multi-byte UTF-8 character crossing the boundary") — these two sites simply never applied that safe pattern.

1. **`src/web/portal/admin/audit.rs:144`** — `truncate(s)` does:
   ```rust
   fn truncate(s: &str) -> &str {
       const MAX: usize = 120;
       if s.len() <= MAX { s } else { &s[..MAX] }   // byte slice, not char-safe
   }
   ```
   It is called from `format_detail` (`audit.rs:131,133,134`) on the audit row's `old_value` / `new_value` columns, which carry admin- and member-entered free text: membership-type / basic-type names (`src/web/portal/admin/types.rs:290,572`), expense category/account names (`src/service/expense_category_service.rs:56`, `src/service/expense_account_service.rs:54`), announcement and event titles (`src/service/announcement_admin_service.rs:125`, `src/service/event_admin_service.rs:159`), settings values such as the org name/description (`src/web/portal/admin/settings.rs:173-174`), member emails (`src/service/member_service/create.rs:43`), and payment descriptions (`src/service/payment_service.rs:183-186`). None of these fields are restricted to ASCII or bounded in length.

   **Trigger:** persist any such value longer than 120 bytes containing a multi-byte character (emoji, accented letter, CJK script) that straddles byte index 120 — something that happens in *ordinary* use, e.g. an org name or announcement title with an emoji, not just a malicious payload. `truncate` is reached by both `GET /portal/admin/audit` (`audit_log_page` → `filtered_entries`, `audit.rs:120`) and `GET /portal/admin/audit/export` (which reuses `truncate` via the same display mapping).

   **Harm:** the slice panics, aborting the request-handling task. There is no `CatchPanicLayer` in the router, so the connection is reset with no HTTP response (effective 500). A single poisoned row makes the audit-log viewer **and** its CSV export un-loadable for **every** admin — a persistent partial denial of service against a security/compliance control — until the offending row is removed directly from the database.

2. **`src/web/portal/admin/announcements.rs:231`** — the admin announcements list builds a preview with:
   ```rust
   let content_preview = if a.content.len() > 100 {
       format!("{}...", &a.content[..100])   // byte slice on announcement body
   } else { a.content.clone() };
   ```
   **Trigger:** an announcement whose body exceeds 100 bytes with a multi-byte character crossing byte index 100. **Harm:** loading `GET /portal/admin/announcements` (`admin_announcements_page`) panics, making the announcements admin list page unreachable while such an announcement exists.

Both are the same root cause (byte-indexed truncation of UTF-8 text) and share one fix. The attacker is at minimum any account that can persist a multi-byte free-text value into one of these fields (admin/staff today); for the audit log the impact is amplified because one record disables the page for all admins. The bug is also reachable by accident through normal non-ASCII content.

## What Changes

- **Add a shared, char-boundary-safe truncate helper** to `src/util/string.rs`:
  ```rust
  /// Truncate `s` to at most `max_chars` characters on a UTF-8 char
  /// boundary, returning a borrowed sub-slice (no allocation). Unlike
  /// `&s[..n]`, this never panics on multi-byte content.
  pub fn truncate_chars(s: &str, max_chars: usize) -> &str {
      match s.char_indices().nth(max_chars) {
          Some((idx, _)) => &s[..idx],
          None => s,
      }
  }
  ```
- **`src/web/portal/admin/audit.rs`** — reimplement `truncate` in terms of `truncate_chars(s, 120)` (or replace its body with the char-boundary match), eliminating the `&s[..MAX]` slice. Behavior is unchanged for ASCII input.
- **`src/web/portal/admin/announcements.rs`** — replace `&a.content[..100]` with `truncate_chars(&a.content, 100)`, appending the `...` ellipsis only when truncation actually occurred (i.e. the returned slice is shorter than the source).
- **Spec updates.** Add a robustness invariant to the `admin-audit-log` capability (the audit viewer and export SHALL NOT panic on multi-byte free-text values) and to the `admin-announcements` capability (the announcements list SHALL NOT panic on multi-byte announcement bodies).
- **Tests.** Unit-test `truncate_chars` on a string whose byte-`max` boundary splits a multi-byte character; regression-test that `truncate`/the announcement preview return a valid (non-panicking) value for such input.

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `admin-audit-log` — adds an invariant that the audit-log viewer and CSV export render multi-byte free-text values without panicking.
- `admin-announcements` — adds an invariant that the admin announcements list renders multi-byte bodies without panicking.

## Impact

- **Code**: ~10 added lines in `src/util/string.rs` (new helper + test); ~3 changed lines in `src/web/portal/admin/audit.rs`; ~3 changed lines in `src/web/portal/admin/announcements.rs`.
- **Wire shape**: no route, request, or response-shape changes. The only observable difference is that previously-panicking requests now return a normal truncated preview.
- **Risk**: low. The change only alters behavior on multi-byte boundary inputs that currently crash; ASCII inputs are byte-for-byte identical to today.
- **Operator follow-up**: if a poisoned `audit_logs` row already exists in a deployed database, the audit page will keep failing until this fix is deployed; after deploy no manual cleanup is required (the row renders safely). The other free-text byte-slice sites that are already safe — `short_id` at `audit.rs:259` (guarded by `len() > 8`, entity_id is always a UUID/`*`) and `&member_id.to_string()[..8]` at `src/web/portal/admin/billing.rs:343` (UUIDs are ASCII) — are intentionally left unchanged.
