# Replacing a submission attachment orphans the old PDF, which then becomes publicly fetchable

## The issue

When a member edits a submission and uploads a new attachment, the previously
stored PDF is never deleted. Nothing else deletes it either, and — worse — the
orphan escapes the authorization gate that protects submission attachments,
because that gate is a lookup against `submissions.attachment_path`, which no
longer names the old file.

The replacement happens in `SubmissionService::update_owned`:

- `src/service/submission_service/mod.rs:200` —
  ```rust
  if let Some(path) = input.new_attachment_path {
      submission.attachment_path = Some(path);
  }
  ```
  The prior `submission.attachment_path` is overwritten and dropped. The
  service never calls `crate::web::uploads::delete_uploaded_file` on it,
  although it already does exactly that on the delete path
  (`src/service/submission_service/mod.rs:247-249`).

- `src/web/portal/submissions.rs:322` — `update_submission` has already
  written the new file to disk via `save_uploaded_document` before the service
  runs, and cleans up only on the *error* path
  (`src/web/portal/submissions.rs:344-345`). On success nothing cleans up the
  replaced file.

Two concrete harms:

1. **The orphan is served without authorization.**
   `GET /uploads/:filename` refuses submission attachments by asking
   `SELECT 1 FROM submissions WHERE attachment_path = ?`
   (`src/web/uploads.rs:270-286`, called at `src/web/uploads.rs:305`). An
   orphaned path matches no row, so `is_submission_attachment` returns
   `false`, `is_private_image` also returns `false` (it only inspects
   `events` and `announcements`), and `serve_upload` streams the PDF to any
   caller — no session, no ownership check, no
   `Content-Disposition: attachment`, no `nosniff`
   (`src/web/uploads.rs:328-356`). The file's content is unchanged member
   proposal material that was never accepted-and-public. Every replaced
   attachment silently converts from "gated" to "anyone with the URL".

2. **Unbounded disk consumption by an authenticated member.**
   The per-member open-submission cap (`MAX_OPEN_SUBMISSIONS = 5`,
   `src/service/submission_service/mod.rs:39`) bounds rows, not bytes. A
   member can POST `/portal/submissions/:id/update` repeatedly against one
   submission, each time attaching a fresh 10 MB PDF
   (`MAX_FILE_SIZE`, `src/web/uploads.rs:25`). Each edit permanently adds
   10 MB of unreferenced data to the uploads directory. Nothing prunes it —
   `delete_uploaded_file` is only ever reached with a path the DB still
   carries.

Attacker: any Active member with submissions enabled. For harm (1) they need
only edit their own submission once; the previously-shared attachment URL
becomes ungated. For harm (2) they loop the edit endpoint.

## Source location

- `src/service/submission_service/mod.rs:200` — the replacement that drops the
  old path without deleting the file.
- `src/web/portal/submissions.rs:309-349` — `update_submission`, the only
  caller of `update_owned` with a `new_attachment_path`.
- `src/web/uploads.rs:270-286` — `is_submission_attachment`, the DB-backed gate
  the orphan escapes.

## Acceptance criteria

Stated against the EXISTING specification — no spec delta is required, because
canon already forbids the resulting behavior and the fix only restores
conformance.

Canonical requirement: **`submissions` → "Attachments are type-restricted,
validated, and served only to authorized viewers"**
(`openspec/specs/submissions/spec.md:128`), which states:

> A submission attachment SHALL be served through an authorization-gated route
> that permits only the submitter and admins — unless the submission has been
> accepted with `public` visibility […] The existing public `/uploads/:filename`
> route SHALL NOT be used to serve non-public attachments.

and its scenario *"A non-owner cannot fetch a private attachment"* ("…including
by guessing its URL" → "the request SHALL be denied").

Also relevant: **`submissions` → "Submission writes are CSRF-protected and
rate-bounded"** (`openspec/specs/submissions/spec.md:161`), whose open-submission
cap exists "to bound spam and storage exhaustion".

The fix is accepted when:

1. After a member replaces a submission's attachment, the previously stored
   file no longer exists on disk under the configured uploads directory.
2. Consequently, `GET /uploads/<old-filename>` returns 404 — the replaced
   attachment is not served by the public route, satisfying "The existing
   public `/uploads/:filename` route SHALL NOT be used to serve non-public
   attachments".
3. The currently-referenced attachment is unaffected: it is still refused by
   `GET /uploads/:filename` and still served by the gated
   `GET /portal/submissions/:id/attachment` route to the submitter and admins,
   with `Content-Disposition: attachment` and `nosniff`.
4. An edit that does NOT supply a new attachment leaves the existing file in
   place and still downloadable through the gated route.
5. Deletion of the replaced file is best-effort in the same sense the delete
   path already is: a filesystem failure is logged and does NOT fail the edit
   or leave the submission row inconsistent.
6. Repeatedly editing one submission with fresh attachments does not grow the
   uploads directory without bound — after N edits, exactly one attachment
   file for that submission remains.
