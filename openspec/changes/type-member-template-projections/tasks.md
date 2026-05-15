## 1. Domain layer

- [ ] 1.1 In `src/domain/member.rs`, add `Copy` to `MemberStatus`'s derive list (alongside the existing `Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq, ToSchema`).
- [ ] 1.2 Add five predicate methods on `MemberStatus`: `pub fn is_active(self) -> bool`, `is_pending`, `is_expired`, `is_suspended`, `is_honorary`. Each implemented as a `matches!(self, MemberStatus::<Variant>)`.
- [ ] 1.3 Add unit tests asserting each predicate returns `true` for its variant and `false` for the other four. Five test cases is fine.

## 2. Template filters

- [ ] 2.1 Create `src/web/templates/filters.rs` with `fmt_long_date(d: &DateTime<Utc>) -> ::askama::Result<String>` formatting via `"%B %d, %Y"`, and `fmt_short_date(d: &DateTime<Utc>) -> ::askama::Result<String>` formatting via `"%b %d, %Y"`.
- [ ] 2.2 Add `fmt_long_date_opt(d: &Option<DateTime<Utc>>)` and `fmt_short_date_opt(d: &Option<DateTime<Utc>>)` returning the empty string when `None`.
- [ ] 2.3 Wire the filters module into Askama's resolution path. Match the pattern used elsewhere in the project (likely a `pub use` in `src/web/templates/mod.rs` or per-template-module imports). Verify on one template that `{{ some_date|fmt_long_date }}` resolves before sweeping the rest.
- [ ] 2.4 Add unit tests asserting each filter produces the expected exact string for a known input timestamp.

## 3. Retype `MemberInfo`

- [ ] 3.1 In `src/web/portal/mod.rs`, change `MemberInfo`'s field types: `id: Uuid`, `status: MemberStatus`, `joined_at: DateTime<Utc>`, `dues_paid_until: Option<DateTime<Utc>>`. Other fields (`username`, `full_name`, `email`, `membership_type`) stay `String`.
- [ ] 3.2 Update the construction site in `src/web/portal/dashboard.rs::member_dashboard`: remove `format!`, `.as_str().to_string()`, and `.to_string()` boilerplate. Pass typed values directly.
- [ ] 3.3 Update the construction site in `src/web/portal/profile.rs`: same simplification.
- [ ] 3.4 Update the construction site in `src/web/portal/security.rs`: same simplification.
- [ ] 3.5 Run `cargo build`. Compiler flags any consumer of the old field types; fix.

## 4. Retype `AdminMemberInfo`

- [ ] 4.1 In `src/web/portal/admin/members.rs`, change `AdminMemberInfo`'s field types: `id: Uuid`, `status: MemberStatus`, `joined_at: DateTime<Utc>`, `dues_paid_until: Option<DateTime<Utc>>`. `initials`, `email`, `username`, `full_name`, `membership_type` stay `String`.
- [ ] 4.2 Update the construction site in `admin_members_page`: remove `format!`, `.as_str().to_string()`, `m.id.to_string()`. Pass typed values directly.
- [ ] 4.3 Run `cargo build`. Fix any remaining drift.

## 5. Migrate templates: status comparisons

- [ ] 5.1 `templates/dashboard/member.html`: replace each `member.status == "..."` with `member.status.is_*()`.
- [ ] 5.2 `templates/portal/profile.html`: same.
- [ ] 5.3 `templates/admin/member_detail.html`: same. Watch for two separate blocks (action-button area and the side-panel status display).
- [ ] 5.4 `templates/admin/members_table.html`: same.
- [ ] 5.5 `templates/admin/members.html`: same.
- [ ] 5.6 `templates/admin/member_new.html`: check whether it uses status comparisons and migrate if so.
- [ ] 5.7 Grep `templates/` for any remaining `member.status == "..."` and migrate stragglers.

## 6. Migrate templates: date filters

- [ ] 6.1 `templates/dashboard/member.html`: every `{{ member.joined_at }}` becomes `{{ member.joined_at|fmt_long_date }}`. Every `{{ member.dues_paid_until }}` (or its `if let Some(...)` form) uses `|fmt_long_date` on the unwrapped value.
- [ ] 6.2 `templates/portal/profile.html`: same long-form treatment.
- [ ] 6.3 `templates/admin/member_detail.html`: same long-form treatment (matching the prior `"%B %d, %Y"` format).
- [ ] 6.4 `templates/admin/members_table.html`: short-form (`fmt_short_date`) — matches the prior `"%b %d, %Y"`.
- [ ] 6.5 `templates/admin/members.html`: confirm format and apply matching filter.
- [ ] 6.6 Grep `templates/` for any remaining `{{ ... .joined_at }}` or `{{ ... .dues_paid_until }}` raw renders and apply the right filter.

## 7. Migrate templates: id rendering

- [ ] 7.1 `id: Uuid` renders to its hyphenated string via `Display`. `{{ member.id }}` continues to produce the same string. Verify this on the admin member detail page (which renders many `{{ member.id }}` instances inside `hx-post` URLs).
- [ ] 7.2 If any template needs a non-default representation of the UUID (none expected), add it as a filter.

## 8. Golden-snapshot tests for the migrated templates

Goal: prove the migrated templates produce byte-equivalent output to today's. Use a golden-snapshot test pattern — the test renders each template with fixture data and compares against a committed `.golden` HTML file. This runs in the autocoder's sandbox without a browser or a running server.

**Important workflow note for the agent**: generate the golden files from the *pre-change* templates first (Section 8.1), commit them, THEN make the field-type changes (Sections 3 and 4) and template migrations (Sections 5–7), THEN re-run the snapshot tests to assert no drift. If the goldens are generated after the change, they capture the new output as the new baseline — which doesn't catch any drift.

- [ ] 8.1 BEFORE making any field-type or template changes, create `tests/member_template_snapshots.rs` that renders each of the seven affected templates (dashboard, profile, security if applicable, admin members.html, admin members_table.html, admin member_detail.html, admin member_new.html if applicable) with a fixed-fixture `MemberInfo` / `AdminMemberInfo` (specific UUID, fixed `joined_at` timestamp, fixed `dues_paid_until`, a representative status for each variant — `Active`, `Pending`, `Expired`, `Suspended`, `Honorary`). Write each rendered output to `tests/snapshots/<template_name>__<status>.golden.html`. Commit these as the baseline.
- [ ] 8.2 Add a test in the same file that re-renders each template with the same fixture and asserts equality with the corresponding `.golden.html` file. Run `cargo test --features test-utils --test member_template_snapshots` and confirm all snapshots match (they will at this point — same code, same goldens just captured).
- [ ] 8.3 Now perform the field-type changes (Section 3, 4) and template migrations (Sections 5, 6, 7).
- [ ] 8.4 Re-run `cargo test --features test-utils --test member_template_snapshots`. Every snapshot SHALL still match — that's the regression net for "rendered HTML is byte-equivalent." Any failure is real drift; fix the change, not the golden.
- [ ] 8.5 Run the full `cargo test --features test-utils` suite. Existing handler-level tests should pass; if any fail, investigate before adjusting the test (a failing test is most likely surfacing a real diff).

## 9. Confirm scope boundaries

- [ ] 9.1 `UserInfo` (in `src/web/templates/mod.rs`) is unchanged — out of scope per design.
- [ ] 9.2 `TypeInfo` and `MembershipTypeInfo` (in `src/web/portal/admin/types.rs`) are unchanged — out of scope per design.
- [ ] 9.3 No new `From<Member> for MemberInfo` impls were added — construction stays per-handler because `membership_type` resolution differs by site.
- [ ] 9.4 The projections still hide `Member` fields (`notes`, `stripe_*`, `discord_id`) from templates — verify by looking at the post-change struct definitions.

## 10. Spec sync

- [ ] 10.1 Confirm the change's delta spec (`openspec/changes/type-member-template-projections/specs/domain-types/spec.md`) matches the implemented behavior.
- [ ] 10.2 At archive time (`opsx:archive`), the new requirements about `MemberStatus` predicates and typed projections merge into `openspec/specs/domain-types/spec.md`.
