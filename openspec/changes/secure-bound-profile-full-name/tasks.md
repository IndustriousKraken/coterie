## 1. Bound the profile-update field

- [ ] 1.1 In `src/web/portal/profile.rs::update_profile`, before building
  `UpdateMemberRequest`: bind `let full_name = form.full_name.trim();`,
  and return the handler's existing inline error fragment (the
  `<div class="p-4 bg-red-50 text-red-800 rounded-md">…</div>` shape its
  `Err` arm already emits, through `crate::web::escape_html`) when
  `full_name.is_empty()` ("Name is required") or when
  `full_name.chars().count() > 200` ("Name is too long (max 200
  characters)"). Use the same 200-character bound `/public/signup` applies
  at `src/api/handlers/public.rs:264`.
- [ ] 1.2 Persist the trimmed value —
  `full_name: Some(full_name.to_string())` — rather than
  `form.full_name.clone()`, so the stored value matches what signup
  stores for the same input.
- [ ] 1.3 Add a doc comment on `update_profile` recording that this is the
  member-side door to `members.full_name` and that its bound intentionally
  mirrors the unauthenticated signup door, so the two cannot drift.

## 2. Regression tests

- [ ] 2.1 Add a test module (or extend the existing one) in
  `src/web/portal/profile.rs` with
  `profile_update_rejects_overlong_full_name`: POST
  `/portal/profile` with a 201-character `full_name` for an Active member
  and assert the response body contains the "too long" message AND that
  `members.full_name` is unchanged in the database.
- [ ] 2.2 Add `profile_update_rejects_blank_full_name`: POST with
  `full_name` of `"   "`, assert the error fragment and that the stored
  name is unchanged.
- [ ] 2.3 Add `profile_update_trims_and_persists_valid_name`: POST with
  `"  Ada Lovelace  "`, assert the `HX-Redirect` success response and that
  the stored `full_name` is exactly `"Ada Lovelace"`.
