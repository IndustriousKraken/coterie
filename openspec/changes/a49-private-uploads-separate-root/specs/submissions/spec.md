# submissions Specification

## MODIFIED Requirements

### Requirement: Attachments are type-restricted, validated, and served only to authorized viewers

An uploaded attachment SHALL be accepted only when its content is confirmed by
authoritative magic-byte inspection (not the client-supplied extension or
content-type) to be a PDF, and SHALL be rejected when it exceeds the size cap.
The stored file SHALL be written under a server-generated name so the uploader's
filename cannot influence the storage path. A submission attachment SHALL be
served through an authorization-gated route that permits only the submitter and
admins — unless the submission has been accepted with `public`
visibility — and SHALL be delivered with `Content-Disposition: attachment` so it
is never rendered inline in the viewer's origin. The existing public
`/uploads/:filename` route SHALL NOT be used to serve non-public attachments.

**Attachments SHALL be stored outside the publicly served uploads root**, in a
private root that no static or public route is mounted on. The public route SHALL
be structurally incapable of reading an attachment, rather than merely instructed
not to.

`GET /uploads/:filename` SHALL serve only from the public root, and SHALL NOT
consult the database to decide whether a submission attachment may be served —
attachments are not in its root, so there is no such decision left for it to make.
The route retains its existing per-file check for non-public event and
announcement images, which remain in the public root and are the deferred
remainder of this pattern.

Deciding an attachment's reachability by querying whether some row still names the
file makes access control a function of referential integrity, which fails open
every way a row can legitimately disappear:

- an attachment replaced during an edit, leaving the previous file unreferenced;
- a member deleted, cascading their submission rows away and unreferencing every
  attachment they had;
- a best-effort unlink failing after its row was already removed;
- the window between writing the file to disk and committing the row that names it.

In each case the file is unchanged and still private, but no row claims it, so a
lookup-based gate publishes it. Storage separation removes the entire class: an
unreferenced file in the private root is wasted disk, not a disclosure.

A private file's reachability SHALL NOT depend on database state.

#### Scenario: A non-PDF or oversized upload is rejected

- **WHEN** a member uploads a file that does not sniff as PDF, or exceeds the
  size cap
- **THEN** the upload SHALL be rejected and no submission attachment SHALL be
  stored

#### Scenario: A non-owner cannot fetch a private attachment

- **WHEN** a member who is neither the submitter nor an admin requests the
  attachment of a submission that is not accepted-and-public (including by
  guessing its URL)
- **THEN** the request SHALL be denied

#### Scenario: Attachments are served as downloads, not inline

- **WHEN** an authorized viewer fetches a submission attachment
- **THEN** the response SHALL carry `Content-Disposition: attachment` and SHALL
  NOT be rendered inline

#### Scenario: An unreferenced attachment is not reachable by the public route

- **WHEN** a stored attachment is no longer named by any `submissions` row — after
  a replacement during an edit, a cascade delete of its submitter, or a failed
  cleanup — and a caller requests it by filename at `/uploads/:filename`
- **THEN** the request SHALL NOT return the file, because the public route serves
  only the public root and the attachment is not in it

#### Scenario: Deleting a member does not publish their attachments

- **WHEN** a member is deleted and their submission rows cascade away
- **THEN** their previously stored attachment files SHALL remain unreachable by
  any public route

#### Scenario: The public upload route makes no attachment decision

- **WHEN** `/uploads/:filename` handles a request
- **THEN** it SHALL resolve the file within the public root and SHALL NOT query
  the `submissions` table; no attachment allow-or-deny lookup SHALL gate the
  response, because attachments are not reachable from that root at all
