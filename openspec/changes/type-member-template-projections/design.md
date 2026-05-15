## Context

The presentation layer for member-related portal pages flattens domain values to strings before handing them to Askama. This is documented in two structs:

```
src/web/portal/mod.rs                MemberInfo            (used by dashboard, profile, security)
src/web/portal/admin/members.rs      AdminMemberInfo       (used by admin members list & detail)
```

Both flatten `status` from `MemberStatus` to `String`, and dates from `DateTime<Utc>` to pre-formatted `String`. Templates then carry the consequences:

- **Status comparisons**: `{% if member.status == "Active" %}` patterns. ~25 occurrences across 7 templates. A typo on either side (a stale variant name in the template, a renamed enum variant in the projection) is silent.
- **Date formatting**: `m.joined_at.format("%B %d, %Y").to_string()` and the short-form `"%b %d, %Y"` are scattered across handlers. A consistency change requires touching every site.

The framework supports better:

- Askama 0.12 supports calling methods on Rust values directly in templates (`{% if member.status.is_active() %}`).
- Askama 0.12 supports custom filters declared in a `filters` module in the same scope as the template struct (`{{ value|fmt_long_date }}`).

This change exercises both.

The projection itself stays — `Member` carries fields (`notes`, `stripe_*`, `discord_id`) that should never reach a member-facing template. The projection is a render-shield. We don't remove it; we type its fields.

## Goals / Non-Goals

**Goals:**
- Status branching in templates is type-checked. A renamed `MemberStatus` variant SHALL force every template branch to update.
- Date formatting lives in one place per format. Choosing between long-form ("September 12, 2025") and short-form ("Sep 12, 2025") is a one-token template-level decision.
- Per-handler `format!`/`.as_str()` plumbing in projection construction sites disappears.
- Wire shape (rendered HTML) is byte-equivalent to today's output for every migrated template.

**Non-Goals:**
- Consolidating `MemberInfo` and `AdminMemberInfo` into one struct. They have different needs (admin carries `initials` + `membership_type` display name; member-facing dashboard / profile doesn't).
- Removing the projection in favor of `&Member`. Render-shield reasons.
- Touching `UserInfo` (the layout-level `current_user` projection in `templates/mod.rs`). That struct only holds id, username, email — no status, no dates. The string-id is mildly inelegant but doesn't have the typo foot-gun.
- Touching `TypeInfo` / `MembershipTypeInfo` projections in `admin/types.rs`. They carry `is_active: bool` (already typed), and their other fields are intentionally pre-formatted display strings (e.g., `fee_dollars: "12.50"`).
- Renaming or restructuring the projection types. `MemberInfo` and `AdminMemberInfo` keep their names so call-site grep is preserved.

## Decisions

### D1. `MemberStatus` gains predicate helpers

Add five small const-equivalent predicates to the existing enum in `src/domain/member.rs`:

```rust
impl MemberStatus {
    pub fn is_active(self)    -> bool { matches!(self, MemberStatus::Active) }
    pub fn is_pending(self)   -> bool { matches!(self, MemberStatus::Pending) }
    pub fn is_expired(self)   -> bool { matches!(self, MemberStatus::Expired) }
    pub fn is_suspended(self) -> bool { matches!(self, MemberStatus::Suspended) }
    pub fn is_honorary(self)  -> bool { matches!(self, MemberStatus::Honorary) }
}
```

Considered: a single `is_status(self, MemberStatus) -> bool` method or expose the enum directly to Askama and use `{% if let MemberStatus::Active = member.status %}`. Rejected — Askama's pattern-matching on enum variants is awkward (no `if let` in expression position in many template engines), whereas method-call branching is idiomatic and reads cleanly.

The predicates are `pub` because templates need them. They take `self` by value because `MemberStatus` is small enough that `Copy` is plausible — adding `Copy` to the existing `#[derive(Debug, Clone, Serialize, ...)]` is a one-token change.

### D2. Add `Copy` to `MemberStatus`

The enum has no heap data; `Copy` lets templates call predicates without `&` ceremony. Required so `member.status.is_active()` compiles when the field is owned (which it will be on the projection).

This is a derive change, not a behavioral change. Existing match arms continue to compile.

### D3. Date filters live in `src/web/templates/filters.rs`

```rust
// src/web/templates/filters.rs
use chrono::{DateTime, Utc};

pub fn fmt_long_date(d: &DateTime<Utc>) -> ::askama::Result<String> {
    Ok(d.format("%B %d, %Y").to_string())
}

pub fn fmt_short_date(d: &DateTime<Utc>) -> ::askama::Result<String> {
    Ok(d.format("%b %d, %Y").to_string())
}

pub fn fmt_long_date_opt(d: &Option<DateTime<Utc>>) -> ::askama::Result<String> {
    Ok(d.map(|x| x.format("%B %d, %Y").to_string()).unwrap_or_default())
}

pub fn fmt_short_date_opt(d: &Option<DateTime<Utc>>) -> ::askama::Result<String> {
    Ok(d.map(|x| x.format("%b %d, %Y").to_string()).unwrap_or_default())
}
```

Considered: one filter parameterized by a format string passed from the template (`{{ d|fmt("%b %d, %Y") }}`). Rejected — that would re-distribute the format-string knowledge instead of consolidating it. Two named filters are easier to standardize on.

Considered: relying on Askama's built-in `format` filter. `format` exists for string formatting, not date formatting; `chrono::DateTime` doesn't expose a `Display` that matches our format anyway. Custom filters are the right shape.

The `_opt` variants exist because some fields (`dues_paid_until`) are `Option<DateTime<Utc>>` and Askama filter dispatch is concrete-typed. The empty-string fallback matches the existing handler behavior (when `dues_paid_until` is `None`, today's projection produces `None: Option<String>`, which templates already handle with `if let Some(...)`). Templates that need to distinguish None from Some will continue to use `{% if let Some(d) = member.dues_paid_until %}{{ d|fmt_long_date }}{% endif %}` — the filter accepts `&DateTime<Utc>`.

### D4. Filters are wired via Askama's `filters` module pattern

Askama 0.12 looks for a `filters` module in scope of the template struct. We declare:

```rust
// In src/web/templates/mod.rs (or per-template-module)
pub use crate::web::templates::filters as askama_filters;
```

…or simply place the `filters` module inside `src/web/templates/mod.rs` and re-export from each template-host module so Askama resolves them. Exact wiring is a small implementation detail that surfaces in the tasks; the principle is "one filters module, every member-context template module imports it."

### D5. `MemberInfo` and `AdminMemberInfo` keep their names and locations

```rust
pub struct MemberInfo {
    pub id: Uuid,
    pub username: String,
    pub full_name: String,
    pub email: String,
    pub status: MemberStatus,
    pub membership_type: String,                 // already a derived display name
    pub joined_at: DateTime<Utc>,
    pub dues_paid_until: Option<DateTime<Utc>>,
}
```

`AdminMemberInfo` mirrors with `initials: String`. Existing call sites construct each struct with field-by-field access; the change replaces `format!(...)` and `.as_str().to_string()` boilerplate with direct field passthrough. No new helpers, no `From<Member>` impl — the construction is per-handler because each one needs the membership-type display name resolved differently (admin pre-fetches a `HashMap<Uuid, String>`, dashboard re-fetches one type at a time).

Considered: a `From<&Member> for MemberInfo` impl. Rejected — `MemberInfo` needs `membership_type: String` which isn't on `Member`; the conversion can't be infallible from `&Member` alone.

### D6. Templates migrate exhaustively in one PR

Mixing typed-and-stringly across templates (some pages use `member.status.is_active()`, others use `member.status == "Active"`) is a maintenance trap — readers won't know which pattern to follow. Every template that consumes `MemberInfo` or `AdminMemberInfo` migrates in this change.

The 7 affected templates:
- `templates/dashboard/member.html`
- `templates/portal/profile.html`
- `templates/portal/security.html` (if it has status comparisons; if not, only date filters apply)
- `templates/admin/members.html`
- `templates/admin/members_table.html`
- `templates/admin/member_detail.html`
- `templates/admin/member_new.html` (if affected)

### D7. Test the predicate helpers and the date filters

- Unit tests for `MemberStatus::is_*()` — five trivial cases, one negative, mostly to lock the canonical names so a future renamer triggers a compile failure rather than a silent test pass.
- Test for each date filter — assert the exact string output for a known input. This is the regression net for "the date format slipped" during the migration.
- Existing handler-level tests render templates end-to-end; they continue to assert the rendered HTML and catch any drift.

## Risks / Trade-offs

- **Risk**: rendered date strings drift from the pre-change output (e.g., one template used `%B %d, %Y` and another silently used `%b %d, %Y`; consolidating to one filter changes the output on a page). → **Mitigation**: take the filter format strings literally from each construction site as it exists today. The two formats stay distinct (`fmt_long_date` for dashboard/profile/admin-detail; `fmt_short_date` for admin-members-table). Spot-check the rendered HTML diff per page.
- **Risk**: Askama's filter resolution doesn't pick up the module via the chosen wiring pattern. → **Mitigation**: confirm the wiring on one template before sweeping the rest. The `filters` module pattern is well-documented in Askama 0.12.
- **Trade-off**: ~5 lines added to `MemberStatus` for predicates. They're trivial and the alternative (templates branching on string) is the foot-gun the change exists to remove.
- **Trade-off**: this is a smaller-payoff change than the previous three proposed. The primary value is template-typo-prevention plus formatter consolidation; both are real but ergonomic. If scope inflates (e.g., into `UserInfo`, `TypeInfo`, or a from-`Member` impl), pull back and ship a tighter version.
- **Trade-off**: `UserInfo` (in the base layout) keeps its stringly-typed `id`. That's deliberate — it doesn't have the status foot-gun and changing it ripples into every page's base context. Out of scope.

## Migration Plan

Single PR; pure-internal (template-and-handler) refactor with byte-equivalent rendered output.

1. Add `MemberStatus::is_*()` predicates and `Copy` derive.
2. Create `src/web/templates/filters.rs` with `fmt_long_date`, `fmt_short_date` and the `_opt` variants.
3. Wire the filters module so the relevant template modules see it.
4. Retype `MemberInfo` fields. Compiler flags every construction site; fix them by removing the `format!` / `.as_str()` boilerplate.
5. Retype `AdminMemberInfo` fields. Compiler flags the construction site in `admin/members.rs`; fix it.
6. Migrate templates one at a time. After each, render a sample page and diff against pre-change output (a tiny shell helper or a one-shot `cargo test` with a snapshot suffices).
7. Run the full test suite; deploy normally.

No DB changes, no config flags, no rollout staging. `git revert` is the rollback.
