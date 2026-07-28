# Bound `full_name` on the profile-update endpoint

## Why

`POST /public/signup` length-bounds its free-text fields — `full_name` at
200 characters (`src/api/handlers/public.rs:264`), per
`openspec/specs/public-signup/spec.md` **Requirement: Signup bounds and
validates its input fields**. The member-facing door to the same column
has no bound at all:

`src/web/portal/profile.rs:86-98`

```rust
pub async fn update_profile(
    State(member_repo): State<Arc<dyn MemberRepository>>,
    Extension(current_user): Extension<CurrentUser>,
    axum::Form(form): axum::Form<UpdateProfileRequest>,
) -> axum::response::Response {
    let update = UpdateMemberRequest {
        full_name: Some(form.full_name.clone()),
        ..Default::default()
    };
    match member_repo.update(current_user.member.id, update).await {
```

`form.full_name` goes straight to the repository. No trim, no emptiness
check, no length check — and none downstream either: `UpdateMemberRequest`
carries no validation and `members.full_name` is an unbounded SQLite
`TEXT`. Nothing between the socket and the column bounds it except the
CSRF layer's form-body cap (`to_bytes(body, 1024 * 1024)`,
`src/api/middleware/security.rs:278`).

So any authenticated Active/Honorary member can `POST /portal/profile`
with a ~1 MB `full_name`, repeatedly. That value is then rendered on the
admin member list and detail pages, on every event and class roster
(`RosterRow::name`, `src/web/portal/admin/events/roster.rs:92`), in the
member CSV export (`src/web/portal/admin/members/bulk.rs:124`), and in
outbound email templates. An empty string is equally accepted, leaving a
nameless row on those same surfaces.

Harm: unbounded per-member storage growth from a single authenticated
endpoint, plus admin surfaces (member list, rosters, CSV export) rendered
unusable by one member's row. This is the same trust-boundary gap the
signup change already closed at the sibling door; closing it here is the
root-cause half.

**This is a contract change**, which is why it is a spec-lane change
rather than an issue. `openspec/specs/member-profile/spec.md` specifies
*which* field the endpoint persists but says nothing about bounds, so
`POST /portal/profile` gains a `400` rejection it does not have today.

## What Changes

- Validate `full_name` in `update_profile` before building
  `UpdateMemberRequest`: trim, reject empty, reject longer than 200
  characters. Same bound and same trimmed-value semantics as
  `/public/signup`, so the two doors to one column agree.
- Persist the trimmed value, matching what signup stores.
- Render the rejection as the endpoint's existing inline error fragment
  (the `p-4 bg-red-50 …` HTML the `Err` arm already returns), so HTMX
  swaps a message rather than showing a raw error.

## Impact

- `src/web/portal/profile.rs` — `update_profile`.
- Spec delta: `openspec/specs/member-profile/spec.md` — one added
  requirement.
- No migration and no data backfill: existing over-long or blank names
  stay as they are; the bound applies to writes from this point on.
