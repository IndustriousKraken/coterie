## Why

`MemberInfo` (in `src/web/portal/mod.rs`) and `AdminMemberInfo` (in `src/web/portal/admin/members.rs`) are template-context projections of `domain::Member`. Today they flatten everything to `String`:

```rust
pub struct MemberInfo {
    pub id: String,
    pub status: String,            // "Active" | "Pending" | "Expired" | …
    pub joined_at: String,          // pre-formatted "%B %d, %Y"
    pub dues_paid_until: Option<String>,
    …
}
```

Templates then compare these strings:

```html
{% if member.status == "Active" %}      …
{% else if member.status == "Pending" %} …
{% else if member.status == "Penidng" %} … <!-- silent typo: branch never fires -->
```

Two concrete consequences:

1. **Typo risk is real and untyped**. There are ~25 `member.status == "..."` comparisons across 7 templates. A typo on either side silently routes a member into the wrong branch with no compile-time signal. This is exactly the foot-gun the F1–F8 domain-types work in `ARCHITECTURE-PUNCHLIST.md` was supposed to eliminate at the domain layer; we lose the protection at the template boundary.
2. **Per-handler date formatting**. Each construction site opens `format!("%B %d, %Y")` or `format!("%b %d, %Y")` inline, so a "what does this date look like?" question becomes "go read every handler that fills a member context." Four sites construct `MemberInfo`; one constructs `AdminMemberInfo` with subtly different format strings.

The fix is *not* "pass `&Member` to templates" — `Member` carries fields that should never reach member-facing templates (`notes`, `stripe_customer_id`, `stripe_subscription_id`, `discord_id`). The projection is a deliberate render-shield. The fix is to *type* the projection: keep the shield, make its fields strongly-typed, and let templates exercise the typed shape.

## What Changes

- **Replace stringly-typed fields in `MemberInfo`** with their domain types:
  - `id: String` → `id: Uuid`
  - `status: String` → `status: MemberStatus`
  - `joined_at: String` → `joined_at: DateTime<Utc>`
  - `dues_paid_until: Option<String>` → `dues_paid_until: Option<DateTime<Utc>>`
  - `username`, `full_name`, `email`, `membership_type` (display name) stay `String` — they're already free-form text.
- **Replace stringly-typed fields in `AdminMemberInfo`** the same way; `initials` (computed) stays `String`.
- **Add helper methods on `MemberStatus`**: `is_active()`, `is_pending()`, `is_expired()`, `is_suspended()`, `is_honorary()`. Templates call these instead of comparing strings.
- **Add Askama date filters** (or template helper functions) for the two formats currently used: `fmt_long_date` (e.g., `"September 12, 2025"`) and `fmt_short_date` (e.g., `"Sep 12, 2025"`). Define them once in a shared place; templates apply them via `{{ member.joined_at|fmt_long_date }}`.
- **Migrate every template** that compares `member.status == "..."` to use the typed helpers (`{% if member.status.is_active() %}`). Migrate every `{{ member.joined_at }}` / `{{ member.dues_paid_until }}` site to apply the date filter.
- **Migrate every projection construction site** (`dashboard.rs`, `profile.rs`, `security.rs`, `admin/members.rs`) to pass typed values straight through — no more `format!`, no more `.as_str().to_string()`.
- **Out of scope for this change**:
  - Consolidating `MemberInfo` and `AdminMemberInfo` into one struct (they have legitimately different needs — admin carries `initials` + `membership_type` display name; member-facing doesn't carry the type).
  - The other admin projections (`TypeInfo`, `MembershipTypeInfo`) — those don't have the status-string foot-gun and aren't on the change's hot path.
  - Removing the projection in favor of `&Member` directly (rejected — the projection is a render-shield against `notes`, `stripe_customer_id`, etc.).

## Capabilities

### New Capabilities

(None — this consolidates and types existing presentation-layer code. No new capability spec.)

### Modified Capabilities
- `domain-types`: adds a requirement that `MemberStatus` exposes typed predicate helpers (`is_active`, `is_pending`, etc.) so callers — especially Askama templates — can branch on status without string comparison.

## Impact

- **Code**: `src/web/portal/mod.rs`, `src/web/portal/admin/members.rs`, `src/web/portal/dashboard.rs`, `src/web/portal/profile.rs`, `src/web/portal/security.rs`. ~4 projection construction sites simplify (no per-field `format!`). One new module or extension to existing template helpers for date filters. ~5 lines added to `MemberStatus` for the predicate helpers.
- **Templates**: ~7 templates change. ~25 `member.status == "..."` comparisons become `member.status.is_*()` calls. ~10 raw date renders pick up `|fmt_long_date` or `|fmt_short_date`. Wire output (the rendered HTML) is byte-identical when the formatter is set up correctly.
- **Tests**: existing handler-level tests assert HTTP responses; they pass without changes. Add unit tests for the `MemberStatus` predicate helpers (trivial). Add a snapshot or integration test for one rendered template per format-filter to catch a regression in the date string.
- **Risk**: low–medium. The risk is "the formatted date came out slightly different than before" — a `Sep 12, 2025` vs `September 12, 2025` mismatch on one page. Mitigation: copy the existing format strings into the filters verbatim; spot-check each migrated template against `git diff` of the rendered output for a sample member.
- **Cost-benefit framing**: this is a smaller-payoff change than the previous three I proposed. The win is mostly typo-prevention plus one source-of-truth for date formatting. Worth doing if the cost stays bounded; not worth scope creep into a larger template-context overhaul.
