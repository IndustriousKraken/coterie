# member-custom-fields

## Why

Orgs carry member attributes Coterie can't know about: a cybersecurity
guild tracks HackTheBox/TryHackMe IDs and LinkedIn profiles; a baduk
club might track a rank; a congregation, a committee. Hardcoding any of
these as first-class member columns is wrong — Coterie is org-agnostic
and "HackTheBox ID" means nothing to most deployments. Today the only
homes for such data are the fixed `member_profiles` columns (bio,
skills, github, blog) and the free-text admin notes field, which is
unstructured and admin-only.

Org-defined custom fields close the gap: the admin defines the fields
once, members fill in their own values from their profile, and admins
see/edit them on the member page. Concretely, this is what lets the
Neon Temple migrate Memberpress metadata (hackthebox_id etc.) off the
old WordPress site before decommissioning it.

## What Changes

- Two tables (migration 039): `member_field_definitions` (name, stable
  `field_key`, type `text|url`, `member_editable`, sort_order,
  is_active) and `member_field_values` (member × field → value, both
  FKs cascading).
- `MemberFieldRepository` + `MemberFieldService`: definition CRUD
  (validated, audited) and value upsert/clear (definition must be
  active; values length-bounded; `url` fields must be http(s) when
  non-empty; blank clears the row).
- Admin management page at `/portal/admin/settings/member-fields`:
  list, create, edit (rename/type/sort/active/member-editable), delete
  (with confirm; values cascade).
- Admin member page gains a Custom Fields card (rendered only when
  active definitions exist) saving all fields in one form.
- Member profile page gains the same card restricted to
  `member_editable` fields — members maintain their own IDs/links.
- Out of scope (deferred): CSV import/export integration, any public or
  directory exposure, richer field types (select, date), per-field
  validation rules. Values are visible to admins and the member only.

## Impact

- Spec: new capability `member-custom-fields` (4 ADDED requirements).
- Code: migration 039; `domain/member_field.rs`;
  `repository/member_field_repository.rs`;
  `service/member_field_service.rs`; ServiceContext + AppState wiring;
  admin settings page + member-detail card + profile card handlers and
  templates.
- Tests: service-level (definition validation + audit, value rules,
  member-editable enforcement, cascade on delete); regenerated
  member-detail and profile template goldens.
