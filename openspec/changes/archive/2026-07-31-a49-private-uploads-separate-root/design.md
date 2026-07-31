# Design — a49-private-uploads-separate-root

## The invariant

**A private file's reachability must not depend on database state.**

Today it does: `serve_upload` decides by querying whether some row still names the
file. That makes access control a function of referential integrity, which is a
category error — foreign keys exist to keep data consistent, not to keep documents
secret. Every way a row can legitimately disappear becomes a disclosure path.

Under the new arrangement the filesystem carries the invariant. A file in the
private root is unreachable by the public route because the public route is not
mounted on that directory. No query, no row, no ordering concern.

## Two roots

```
{data_dir}/uploads/          public  — served by GET /uploads/:filename
{data_dir}/private-uploads/  private — no route mounted; read only by gated handlers
```

`GET /uploads/:filename` keeps its path-traversal guard and then serves from the
public root. It stops querying `submissions` altogether — not because it is told
not to serve attachments, but because there is no attachment there to serve. That
is the whole point: the decision is removed rather than made more carefully.

Its per-file check for members-only images survives this change, since those files
stay in the public root for now. So the route does not become lookup-free here; it
becomes lookup-free *for the class this change moves*, and the remaining lookup is
the visible marker of the deferred half.

Gated handlers (today: the submission-attachment route) resolve their file inside
the private root and apply their existing authorization plus
`Content-Disposition: attachment` and `nosniff`.

## Why not keep one root and fix the orphan

Because the orphan is one of four. Replacement, cascade delete, failed unlink, and
the write-before-commit window all produce the same end state — a file on disk
that no row names — and the denylist publishes all four identically. Patching the
first leaves the other three, and leaves the next contributor to discover the
shape by writing a fifth.

The cascade case is the one that should settle it: `ON DELETE CASCADE` on
`submitter_member_id` means **removing a member publishes their PDFs**. No
reasonable reading of "delete this member" includes "and make their private
documents world-readable." That is not a bug in the delete path; it is the storage
design asserting itself.

## Why not a denylist plus an allowlist, or a "private" flag column

A flag is the same design with a different spelling: it still requires a lookup,
still fails open when the row is gone, and still costs a query per public asset.
Any scheme where the answer to "may I serve this?" lives in the database inherits
the whole class.

The filesystem already offers exactly the primitive needed — a namespace the
public handler cannot address — for free.

## Migration

1. Create the private root.
2. Move every file named by `submissions.attachment_path` from the public root
   into it.
3. Rewrite those `attachment_path` values to the new prefix.
4. Files in the public root that no row names are **left alone**. They are today's
   orphans; this change does not attempt to identify or delete them, because
   distinguishing "orphaned attachment" from "legitimately public upload" after
   the fact is guesswork. They stay public, which is their current state — the
   change stops *new* ones from being created.

Point 4 is a deliberate limitation and worth stating plainly: **any attachment
already orphaned before this lands stays publicly fetchable.** If that matters for
this deployment, the remedy is an operator-run audit of the uploads directory
against known-good paths, not an automated guess.

## Scope: attachments now, images later

Members-only event and announcement images have the same weakness. They are not
moved here because their visibility is **mutable** — an admin can flip an event
between `Public` and `MembersOnly` at any time — so a correct move needs
semantics for relocating the file on transition, plus a decision about what
happens to a cached public URL when an event becomes members-only.

Submission attachments have no such problem: an attachment is private for its
whole life, and the one public case (accepted-with-public-visibility) is already
served through the gated route rather than by a direct URL.

Doing the easy half now is not the same as pretending the other half is fine. The
proposal names the images case explicitly so it is tracked rather than forgotten.
