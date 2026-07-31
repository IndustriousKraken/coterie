# Tasks

The goal is that the public route **cannot** reach a private file, not that it is
told not to. If an implementation step ends with a database lookup deciding
whether to serve, it has reproduced the bug in a new spelling.

## 1. Private root

- [x] 1.1 `src/config/mod.rs`: add `private_uploads_path()` alongside
  `uploads_path()`, defaulting to `{data_dir}/private-uploads`. Same
  data-dir-relative convention, so backup and restore pick it up without a
  separate rule.
- [x] 1.2 Verify the deploy docs' backup script covers `{data_dir}` wholesale
  rather than the uploads directory by name — if it names `uploads` explicitly,
  the private root would silently go unbacked-up. Fix it if so.

## 2. Public route stops deciding

- [x] 2.1 `src/web/uploads.rs`: delete `is_submission_attachment` entirely.
- [x] 2.2 `serve_upload`: keep the path-traversal guard, resolve within the public
  root, and remove the submission lookup.
- [x] 2.3 Do not replace it with a flag column or a "is this private" query for
  attachments. Attachments are gone from this root; there is no decision to make.

## 2b. Invert the image predicate (fail closed)

- [x] 2b.1 Replace `is_private_image` with `is_public_image`: match
  `events.image_url` where `visibility = 'Public'`, UNION
  `announcements.image_url` where `is_public = 1`. Positive phrasing only.
- [x] 2b.2 `serve_upload` serves when the allow-list matches, and otherwise
  requires an authenticated session — so a deleted, cascaded, or not-yet-committed
  row denies instead of publishes.
- [x] 2b.3 Do NOT move image files. Visibility is mutable; the query returning a
  different answer for the same file is the whole point, and relocating on every
  transition would leave a window and a stale URL each time.
- [x] 2b.4 Confirm the allow-list is exhaustive before shipping: after task 3, the
  only writers into the public root are `admin/events/single.rs` and
  `admin/announcements.rs`. If a third appears later it must be registered here,
  and its omission fails visibly (broken asset) rather than silently.

## 3. Write and read attachments in the private root

- [x] 3.1 `save_uploaded_document` call sites for submissions pass the private
  root. The helper already takes a directory parameter, so this is a call-site
  change, not a signature change.
- [x] 3.2 The gated attachment route resolves within the private root.
- [x] 3.3 `delete_uploaded_file` call sites for attachments pass the private root.
- [x] 3.4 Store `attachment_path` with a prefix that distinguishes the root, so a
  path alone is unambiguous about where it lives.

## 3b. Delete the replaced attachment on edit (absorbed from the reported issue)

This is the disk-leak half of
`issues/fix-orphaned-submission-attachment-on-replace`, folded in so that issue
can be dropped rather than landed. After the storage split it is hygiene, not a
security control — which is why best-effort deletion is acceptable here and was
not before.

- [x] 3b.1 `SubmissionService::update_owned` takes an uploads-root parameter,
  mirroring the signature `delete(&self, actor_id, id, uploads_dir)` already uses.
  The service owns delete-path cleanup today; it owns this one too rather than
  duplicating the rule in the handler.
- [x] 3b.2 Capture the pre-edit path before overwriting `attachment_path`.
- [x] 3b.3 After the row update succeeds — and only when a new attachment was
  supplied, the previous path was `Some`, and the two differ — best-effort delete
  the old file. Order matters: if the update fails, the old file is still the
  referenced one and must survive.
- [x] 3b.4 `src/web/portal/submissions.rs::update_submission` passes the root
  through.
- [x] 3b.5 Document it as reclaiming disk. Do NOT write that the orphan "would
  otherwise be served ungated" — after this change that is false, and a wrong
  security rationale in a comment outlives the person who wrote it.
- [x] 3b.6 Test: replacing an attachment leaves exactly one file on disk.

## 4. Migration

- [x] 4.1 Create the private root at startup if absent, mirroring how the uploads
  directory is created.
- [x] 4.2 Move each file named by `submissions.attachment_path` from the public
  root into the private root, then rewrite the stored path. Order matters: move
  first, then rewrite, so an interrupted run leaves rows pointing at files that
  still exist.
- [x] 4.3 Make it idempotent — a re-run with paths already rewritten must be a
  no-op, not an error.
- [x] 4.4 Leave unreferenced files in the public root untouched. Distinguishing an
  already-orphaned attachment from a legitimately public upload after the fact is
  guesswork. **Log a count of files in the public root that no row names**, so the
  operator learns whether a manual audit is warranted for this deployment.

## 5. Tests

- [x] 5.1 An attachment's filename requested at `/uploads/:filename` is not served
  — with its `submissions` row present.
- [x] 5.2 Same request after the row is deleted — still not served. This is the
  regression that the whole change exists for; assert it explicitly rather than
  trusting the directory split.
- [x] 5.3 Deleting a member cascades their submissions away and their attachment
  remains unreachable by the public route.
- [x] 5.4 The gated route still serves the attachment to the owner and to an admin,
  with `Content-Disposition: attachment` and `nosniff` intact.
- [x] 5.5 A public upload (an event image on a `Public` event) is still served
  normally by `/uploads/:filename`.
- [x] 5.6 Assert `serve_upload` no longer queries `submissions` — a test that
  fails if someone reintroduces an attachment lookup.
- [x] 5.8 Deleting a members-only event leaves its image **unreachable**
  anonymously. This is the inversion's regression test: under the old deny-list it
  became public, under the allow-list it must not.
- [x] 5.9 Flipping an event `Public` → `MembersOnly` → `Public` changes anonymous
  reachability both ways with no file movement.
- [x] 5.7 Migration idempotency: run it twice, second run is a no-op.

