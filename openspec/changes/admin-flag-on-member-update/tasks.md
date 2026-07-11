# Tasks

## 1. Domain + repository

- [x] 1.1 Add `is_admin: Option<bool>` to `UpdateMemberRequest` with a
  doc note that `MemberService::update` handles it via
  `MemberRepository::set_admin` (the generic repo `update` does not
  write it).
- [x] 1.2 Add `MemberRepository::count_admins() -> Result<i64>` (trait +
  Sqlite impl) for the last-admin guard.

## 2. Service

- [x] 2.1 In `MemberService::update`, when `request.is_admin` differs
  from the member's current flag: reject revoking when
  `count_admins() <= 1`; otherwise call `set_admin` BEFORE the generic
  repo update (so the `MemberUpdated` event payload carries the new
  flag) and write a `grant_admin` / `revoke_admin` audit entry with
  old/new values. Unchanged flag → no admin audit row.

## 3. Handler + template

- [x] 3.1 `AdminUpdateMemberForm` gains `is_admin: Option<String>`
  (checkbox), mapped as `Some(form.is_admin.is_some())` — same pattern
  as `bypass_dues`.
- [x] 3.2 `AdminMemberDetailInfo` carries `is_admin`; the template
  renders an "Administrator" checkbox beside "Bypass dues" and the
  `Include "ADMIN"` hint is deleted.

## 4. Tests

- [x] 4.1 Grant via `MemberService::update` sets the flag and writes a
  `grant_admin` audit row.
- [x] 4.2 Revoke with two admins succeeds (`revoke_admin` audited);
  revoking the last admin returns an error and leaves the flag set.
- [x] 4.3 Notes containing "ADMIN" (with `is_admin: None`) change
  nothing — the old fictional mechanism stays dead.
- [x] 4.4 `is_admin` equal to the current value writes no
  grant/revoke audit row.
