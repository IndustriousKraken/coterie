# admin-flag-on-member-update

## Why

There is no working way to make a member an administrator from the
portal. The member update form's notes field hints `Include "ADMIN" to
grant admin privileges` (`templates/admin/member_detail.html:160`,
shipped with the original member-edit scaffold), but nothing has ever
parsed it: the handler stores notes verbatim, `UpdateMemberRequest` has
no admin field, and `MemberRepository::set_admin`
(`member_repository.rs:78`) has zero callers. An operator who follows
the hint gets a member with "ADMIN" in their notes and no admin access
(observed in production). The only real admin paths are the first-boot
`/setup` page and the `create_admin` CLI — both bootstrap-only.

## What Changes

- Replace the fictional notes hint with a real **Administrator
  checkbox** on the member update form, carried as `is_admin` through
  `MemberService::update` (the mandated single entrypoint for
  admin-driven member mutations), which calls the existing
  `MemberRepository::set_admin`.
- Guard: revoking the flag from the **last remaining administrator** is
  rejected. A zero-admin database both locks every operator out and
  re-arms the unauthenticated `/setup` page on restart (see
  `api/middleware/setup.rs`) — refusing the demotion closes both.
- Distinct audit entries (`grant_admin` / `revoke_admin`) with
  old/new values, separate from the generic `update_member` row.
- Takes effect on the member's next request — the auth middleware reads
  `is_admin` from the DB per request (`api/middleware/auth.rs:77`), so
  no session invalidation or re-login is needed.
- Notes remain plain notes; text content never affects adminness.

## Impact

- Spec: `admin-members` — one ADDED requirement.
- Code: `domain::UpdateMemberRequest` (+`is_admin: Option<bool>`),
  `MemberService::update` (guard + set_admin + audit),
  `MemberRepository` (+`count_admins`), update handler form field,
  member detail template (checkbox in, lying hint out).
- Tests: service-level grant / revoke / last-admin guard / notes-text
  inertness / no-change-no-audit.
