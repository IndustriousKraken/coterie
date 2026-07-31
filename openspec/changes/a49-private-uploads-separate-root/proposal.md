# a49-private-uploads-separate-root

## Why

Coterie stores every uploaded file in one directory and serves that directory
publicly at `GET /uploads/:filename`. Files that must NOT be public are protected
by asking the database whether the filename is spoken for:

```rust
// src/web/uploads.rs
if is_submission_attachment(&db_pool, &filename).await { return NOT_FOUND; }
if is_private_image(&db_pool, &filename).await { /* require a session */ }
// otherwise: stream it to anyone
```

That is a **denylist over a public-by-default directory**, and it fails open. The
default outcome for an unrecognised file is disclosure, and recognition depends on
a database row continuing to exist. When the row goes, the protection goes — and
it always goes in the direction of more exposure.

The row goes in at least four ways:

1. **Attachment replaced on edit.** `update_owned` overwrites
   `submission.attachment_path` without deleting the old file. The orphan now
   matches no row and is served to anyone. (Reported as
   `fix-orphaned-submission-attachment-on-replace`.)
2. **Member deleted.** `submissions.submitter_member_id` is
   `REFERENCES members(id) ON DELETE CASCADE`. Removing a member deletes their
   submissions and **publishes their attachment PDFs**. Deleting a member is
   supposed to make their data less reachable, not more.
3. **Cleanup fails.** Both existing delete paths are best-effort
   (`let _ = delete_uploaded_file(...)`). A failed unlink after a successful row
   delete leaves a file that is now public.
4. **The write/commit window.** The file is written to disk before the row is
   committed. In that window it is a public URL.

The current design's own doc comment says *"relying on the UUID being unguessable
is not access control"* — while the scheme it defends relies on a database row
being present for access control. The reachability of a private document should
not be a function of referential integrity.

Fixing the reported orphan bug does not fix this. It closes one of four holes and
leaves the shape that produces them.

### Why a structural fix is cheaper than the denylist

Separating the storage roots is less code, not more. It **deletes**
`is_submission_attachment` entirely, halves the database round-trips on every
public asset request (today every marketing-calendar image costs two queries; one
remains until the deferred images move lands), and removes the requirement that
every future private-file feature remember to add its own denylist entry — a
footgun whose failure mode is silent publication.

After the change, an orphaned attachment is a wasted disk block. That is what it
should always have been.

## What Changes

- **Two storage roots.** A public root (today's uploads directory) and a private
  root that no static or public route is mounted on.
- **`GET /uploads/:filename` serves only the public root.** It cannot reach an
  attachment, regardless of what the database says or does not say, so it stops
  querying `submissions` entirely. Its per-file check for members-only images
  stays until those move too.
- **Submission attachments are written to the private root** and remain served
  exclusively by their existing authorization-gated route, which already enforces
  owner/admin/accepted-public access plus `Content-Disposition: attachment` and
  `nosniff`.
- **`is_submission_attachment` is deleted.** Its job is done by the filesystem.
- **A migration moves existing attachments** into the private root and rewrites
  the stored `attachment_path` values.
- **The orphan-on-replace bug is still fixed**, but as disk hygiene rather than as
  a security control — see the sequencing note below.

## Impact

- **Spec:** MODIFIED `submissions` — the attachment requirement gains the
  structural storage rule alongside its existing serving rule.
- **Code:** `src/config/mod.rs` (private uploads path), `src/web/uploads.rs`
  (public route loses both lookups; save/delete helpers take a root),
  `src/service/submission_service/` and `src/web/portal/submissions.rs` (write and
  read from the private root), a migration to move files and rewrite paths.
- **Net code change is negative** — a function and two query paths removed.
- **Images are fixed by inverting the predicate, not by moving files.**
  `is_private_image` becomes `is_public_image`: the route asks whether a file is
  known public and refuses everything else. Same cost, opposite failure direction
  — a vanished row now denies instead of publishes. This is the right mechanism
  for images precisely because their visibility is **mutable**: an event can flip
  between `Public` and `MembersOnly`, and a query returns a different answer for
  the same file with no relocation, no path rewrite, and no stale-URL window.
  Attachments get storage separation because their privacy is fixed for life;
  images get a fail-closed query because theirs is a property of a row.
- **The allow-list is complete, which is what makes it safe.** After attachments
  move, the public root has exactly two writers — event images
  (`admin/events/single.rs`) and announcement images (`admin/announcements.rs`) —
  and both are fully represented in the `events` and `announcements` tables. There
  is no third category of public upload that a deny-by-default rule would break.

## Supersedes the reported issue

`issues/fix-orphaned-submission-attachment-on-replace` (PR #140) is **absorbed by
this change** and should not land separately. Its disk-leak fix is folded into the
tasks here, so nothing is lost by dropping it.

It should not land on its own because both of its substantive instructions become
wrong once storage is separated:

- Its task 1.4 asks the implementer to write into a doc comment that an orphan
  "would be served ungated by `GET /uploads/:filename`". After this change that
  sentence is false, and a freshly-written but incorrect security rationale is
  worse than no comment at all.
- Its task 1.3 specifies a best-effort delete. Under today's design a failed
  unlink is a silent disclosure, so best-effort is not defensible; only after this
  change is it the right error handling.

Note also that deleting the proposal does not delete the finding: the underlying
code defect is real until this change lands, so an audit re-run would legitimately
rediscover it. Landing `a49` is what makes the finding moot.
