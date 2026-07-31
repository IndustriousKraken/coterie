# Tasks

## 1. Delete the replaced attachment when an edit swaps it out

- [ ] 1.1 In `src/service/submission_service/mod.rs`, give
  `UpdateSubmissionInput` handling in `update_owned` access to the uploads
  root: add an `uploads_dir: &str` parameter to `update_owned` (mirroring the
  signature `delete(&self, actor_id, id, uploads_dir: &str)` already uses at
  line 229), so the service — which already owns the delete-path cleanup — owns
  this one too rather than duplicating the rule in the handler.
- [ ] 1.2 In `update_owned`, capture the pre-edit path before overwriting it:
  bind `let previous = submission.attachment_path.clone();` before the
  `if let Some(path) = input.new_attachment_path` block at line 200.
- [ ] 1.3 After `self.submission_repo.update(submission).await` succeeds, and
  ONLY when a new attachment was supplied AND `previous` is `Some` AND
  `previous != the new path`, best-effort delete the old file with
  `let _ = crate::web::uploads::delete_uploaded_file(uploads_dir, &previous).await;`
  — the same best-effort call and rationale as `delete` at line 248. Deleting
  after the row update is deliberate: if the update fails, the old file is
  still the referenced one and must survive.
- [ ] 1.4 Update the `update_owned` doc comment to state that a replaced
  attachment is deleted, and why (an orphaned file is no longer matched by
  `is_submission_attachment` and would be served ungated by
  `GET /uploads/:filename`).

## 2. Pass the uploads root from the handler

- [ ] 2.1 In `src/web/portal/submissions.rs::update_submission`, pass the
  already-computed `uploads_dir` (line 321) into the `update_owned` call at
  line 338. No new extractor is needed — `State<Arc<Settings>>` is already in
  the handler's signature.
- [ ] 2.2 Confirm the existing error-path rollback
  (`delete_if_upload(&uploads_dir, form.attachment_path.as_deref())`, lines
  344-345) is unchanged: on a failed edit the NEW file is removed and the old
  one is kept.

## 3. Tests

- [ ] 3.1 In `src/service/submission_service/tests.rs`, add
  `replacing_an_attachment_deletes_the_previous_file`: create a submission with
  attachment A on a temp uploads dir, call `update_owned` with a new attachment
  B, then assert the file for A no longer exists on disk, the file for B does,
  and the stored `attachment_path` is B's.
- [ ] 3.2 Add `editing_without_a_new_attachment_keeps_the_existing_file`:
  call `update_owned` with `new_attachment_path: None` and assert the original
  file is still on disk and still referenced by the row.
- [ ] 3.3 Add `replacing_an_attachment_survives_a_missing_old_file`: delete
  A from disk manually before the edit, then assert `update_owned` still
  returns `Ok` and the row points at B — the cleanup is best-effort and must
  not fail the edit.
- [ ] 3.4 Add an integration test
  `replaced_attachment_is_not_served_by_the_public_uploads_route` (new file
  `tests/submission_attachment_orphan_test.rs`, following the router setup in
  the existing submissions integration test): after an authenticated member
  replaces their attachment, assert `GET /uploads/<old-filename>` returns 404
  and `GET /uploads/<current-filename>` still returns 404 (refused by
  `is_submission_attachment`), while
  `GET /portal/submissions/:id/attachment` as the submitter returns 200.
- [ ] 3.5 Run `cargo test` and confirm the new tests pass with no existing
  submissions test regressed.
