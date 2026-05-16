## ADDED Requirements

### Requirement: MemberStatus exposes typed predicate helpers

`domain::MemberStatus` SHALL expose `pub fn is_active(self) -> bool`, `is_pending`, `is_expired`, `is_suspended`, and `is_honorary` predicate methods. These methods exist so callers — especially Askama templates rendering `MemberStatus` values from `MemberInfo` / `AdminMemberInfo` projections — can branch on status without comparing against string literals. A typo in a predicate name SHALL be caught at compile time; a renamed `MemberStatus` variant SHALL force every predicate to be updated.

`MemberStatus` SHALL also derive `Copy` so the predicates can be called on owned values without `&` ceremony at the template call site.

#### Scenario: Templates branch on typed predicates

- **WHEN** a template renders a member context and needs to branch on the member's status
- **THEN** it SHALL call `member.status.is_active()` (or the relevant predicate) rather than comparing against a string literal like `member.status == "Active"`

#### Scenario: Renaming a variant catches drift at compile time

- **WHEN** a contributor renames `MemberStatus::Pending` (e.g., to `Submitted`)
- **THEN** the corresponding `is_pending` predicate SHALL be updated as part of the rename or the build SHALL fail; templates calling `is_pending` SHALL fail to render against the new variant set, surfacing the drift immediately rather than silently routing the wrong branch

#### Scenario: Predicates are total over the variant set

- **WHEN** a contributor adds a new variant to `MemberStatus`
- **THEN** they SHALL also add the corresponding `is_<variant>` predicate so every variant is observable from templates without falling back to string comparison

### Requirement: Member-context template projections carry typed status and dates

The presentation projections `MemberInfo` (in `src/web/portal/mod.rs`) and `AdminMemberInfo` (in `src/web/portal/admin/members.rs`) SHALL hold:

- `id: Uuid` (not `String`)
- `status: MemberStatus` (not `String`)
- `joined_at: DateTime<Utc>` (not pre-formatted `String`)
- `dues_paid_until: Option<DateTime<Utc>>` (not `Option<String>`)

Date formatting SHALL be applied at the template layer via Askama filters (`fmt_long_date`, `fmt_short_date`, plus `_opt` variants for `Option<DateTime<Utc>>`). Per-handler `format!("%B %d, %Y")` calls in projection construction sites SHALL be removed.

The projections SHALL remain distinct types from `domain::Member` so that `Member` fields not safe to render to member-facing pages (`notes`, `stripe_customer_id`, `stripe_subscription_id`, `discord_id`) cannot leak into a template via the projection.

#### Scenario: Construction sites pass typed values directly

- **WHEN** a handler constructs `MemberInfo` from a `Member`
- **THEN** the construction SHALL move `member.status`, `member.joined_at`, `member.dues_paid_until`, and `member.id` straight onto the projection without per-field `format!` or `.as_str()` conversion

#### Scenario: Templates format dates via the filter, not via pre-formatted strings

- **WHEN** a template renders `member.joined_at`
- **THEN** it SHALL apply the appropriate filter (`{{ member.joined_at|fmt_long_date }}` or `|fmt_short_date`); the projection SHALL NOT carry a pre-formatted string

#### Scenario: Adding a new render-shielded field to Member does not leak

- **WHEN** a contributor adds a new field to `domain::Member` (e.g., a private admin note)
- **THEN** the field SHALL NOT automatically appear on `MemberInfo` / `AdminMemberInfo`; the contributor must explicitly add it to a projection if it should render
