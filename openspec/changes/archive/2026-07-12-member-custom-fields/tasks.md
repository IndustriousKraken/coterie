# Tasks

## 1. Schema + domain + repository

- [x] 1.1 Migration 039: `member_field_definitions` +
  `member_field_values` (both value FKs `ON DELETE CASCADE`; unique
  name and field_key; CHECK on field_type).
- [x] 1.2 `domain/member_field.rs`: `MemberFieldDefinition`,
  `MemberFieldValue`, `FieldWithValue` (definition + optional value for
  display), create/update request structs.
- [x] 1.3 `repository/member_field_repository.rs`: trait + Sqlite impl —
  definition CRUD, `fields_with_values(member_id, include_inactive:
  false)` (LEFT JOIN, sort_order), `set_value` upsert / delete-on-blank.

## 2. Service + wiring

- [x] 2.1 `service/member_field_service.rs`: definition CRUD with
  validation (name ≤100 non-empty, key slug-format unique, type
  text|url) and audit; `save_values(actor, member_id, pairs,
  member_scope: bool)` enforcing active-definition, 500-char bound,
  url-prefix rule, blank-clears, and (member_scope) member_editable.
- [x] 2.2 Wire repo + service through ServiceContext and AppState
  FromRef.

## 3. Admin management UI

- [x] 3.1 GET/POST `/portal/admin/settings/member-fields` (+ per-field
  update/delete posts) with template — list, inline create, edit,
  delete-with-confirm.

## 4. Value editing UIs

- [x] 4.1 Admin member page: Custom Fields card + POST
  `/portal/admin/members/:id/custom-fields`.
- [x] 4.2 Member profile page: card restricted to member-editable
  fields + POST `/portal/profile/custom-fields`.

## 5. Tests

- [x] 5.1 Service: definition validation + audit; duplicate key
  rejected.
- [x] 5.2 Service: value rules — inactive definition rejected, url
  validation, 500-char bound, blank clears, upsert overwrites.
- [x] 5.3 Service: member_scope rejects non-member-editable writes;
  admin scope allows them.
- [x] 5.4 Cascade: deleting a definition removes its values; deleting a
  member removes their values.
- [x] 5.5 Regenerate member-detail + profile template goldens.
