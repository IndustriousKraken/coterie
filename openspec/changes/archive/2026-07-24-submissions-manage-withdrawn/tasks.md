# Tasks

Both actions are owner-scoped, state-guarded, and CSRF-protected. Deletion is
destructive — enforce the state guard and ownership on the server, never trust
the button's presence.

## 1. Service — state machine + cleanup

- [x] 1.1 `src/service/submission_service/`: add `delete(member_id, id)` — verify
  the caller is the submitter, verify status is `withdrawn` or `declined`, delete
  the attachment via `delete_uploaded_file` (best-effort), then delete the row.
  Refuse (no-op error) for any other state or a non-owner.
- [x] 1.2 Add `reopen(member_id, id)` — verify submitter + status `withdrawn`,
  transition to `submitted`. Refuse otherwise.

## 2. Web — routes + UI

- [x] 2.1 `src/web/portal/submissions.rs`: owner-scoped `POST
  /portal/submissions/:id/delete` and `POST /portal/submissions/:id/reopen`
  (CSRF-protected), calling the service actions.
- [x] 2.2 `templates/portal/submissions.html` / `submission_detail.html`: show a
  **Delete** button on `withdrawn`/`declined` rows and a **Re-open** button on
  `withdrawn` rows. Do not show them on active states.

## 3. Tests

- [x] 3.1 Owner deletes a `withdrawn` (and a `declined`) submission → row gone,
  attachment deleted.
- [x] 3.2 Delete of a `submitted`/`under_review`/`accepted`/`scheduled` submission
  → refused, unchanged.
- [x] 3.3 Owner re-opens a `withdrawn` → status `submitted` and editable; re-open
  of a non-`withdrawn` → refused.
- [x] 3.4 A non-owner's delete/re-open → denied without disclosure.

## 4. Verify

- [x] 4.1 `openspec validate submissions-manage-withdrawn --strict` passes.
- [x] 4.2 `cargo test` (submissions suite) green; `cargo clippy` clean.
