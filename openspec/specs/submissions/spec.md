# submissions Specification

## Purpose
TBD - created by archiving change member-proposal-submissions. Update Purpose after archive.
## Requirements
### Requirement: Member submissions are gated by an org toggle, default off

The system SHALL provide a boolean organization setting `submissions.enabled`
defaulting to `false`. When it is `false`, the submission routes SHALL NOT be
mounted and no submission UI SHALL appear in the portal, so an org that does not
opt in has no added surface. When `true`, an authenticated member SHALL be able
to create a submission carrying a title, an abstract, a requested visibility of
`public` or `members`, an optional PDF attachment, and an optional preferred
date and duration. A new submission SHALL be persisted with status `submitted`
and its submitter set to the authenticated member. Title and abstract SHALL be
length-bounded and rejected when over the limit.

#### Scenario: Disabled capability exposes no surface

- **WHEN** `submissions.enabled` is `false` and any caller requests a submission
  route
- **THEN** the route SHALL NOT be served (as if it does not exist) and the
  portal SHALL show no submission entry point

#### Scenario: A member creates a submission

- **WHEN** `submissions.enabled` is `true` and an authenticated member posts a
  valid title and abstract
- **THEN** a submission SHALL be persisted with status `submitted` and its
  submitter set to that member's id

#### Scenario: Oversized fields are rejected

- **WHEN** a member submits a title or abstract longer than the configured bound
- **THEN** the request SHALL be rejected with a validation error and no row SHALL
  be persisted

### Requirement: A member can access only their own submissions

A member SHALL be able to read, edit, withdraw, delete, and re-open ONLY submissions whose submitter is that same authenticated member. Editing SHALL be permitted only while the submission is `submitted`. The owner MAY DELETE a submission that is in a terminal `withdrawn` or `declined` state — removing it from their list and deleting its attachment, if any; deletion of a submission in any other state (`submitted`, `under_review`, `accepted`, `scheduled`) SHALL be refused. The owner MAY RE-OPEN a `withdrawn` submission back to `submitted` for revision and resubmission; re-open SHALL be allowed only from `withdrawn` (a `declined` submission is not resurrected). A request by any member to read or mutate a submission they do not own SHALL be denied without revealing the resource, and MUST NOT rely on the id being unguessable. Admins are exempt from the ownership restriction for review purposes.

#### Scenario: Cross-member read is denied

- **WHEN** member A requests submission `X` owned by member B (by its id)
- **THEN** the response SHALL be denied (404 or 403) and SHALL NOT disclose `X`'s
  contents

#### Scenario: Owner reads and withdraws their own submission

- **WHEN** member B requests or withdraws submission `X` that B submitted
- **THEN** the operation SHALL succeed, and after withdrawal `X` SHALL be in a
  terminal `withdrawn` state

#### Scenario: Editing after a decision is refused

- **WHEN** a member edits a submission that is no longer `submitted`
- **THEN** the edit SHALL be refused

#### Scenario: Owner deletes a withdrawn or declined submission

- **WHEN** member B deletes their own submission that is `withdrawn` or `declined`
- **THEN** the submission SHALL be removed from their list and its attachment (if
  any) SHALL be deleted

#### Scenario: Deleting a non-terminal submission is refused

- **WHEN** member B attempts to delete their own submission that is `submitted`,
  `under_review`, `accepted`, or `scheduled`
- **THEN** the deletion SHALL be refused and the submission SHALL be unchanged

#### Scenario: Owner re-opens a withdrawn submission

- **WHEN** member B re-opens their own `withdrawn` submission
- **THEN** the submission SHALL return to `submitted` and become editable again;
  re-opening a submission that is not `withdrawn` SHALL be refused

#### Scenario: A non-owner cannot delete or re-open a submission

- **WHEN** member A attempts to delete or re-open submission `X` owned by member B
- **THEN** the request SHALL be denied without disclosing `X`

### Requirement: Publication and scheduling are reviewer-gated

A member's requested visibility SHALL be treated as a request only; NO
submission content SHALL reach the public/marketing surface until an admin
accepts it. An admin SHALL move a submission through
`submitted → under_review → accepted | declined`. Aside from the submitter
withdrawing their own submission, or re-opening their own `withdrawn` submission
back to `submitted` (both per the owner-access requirement), no non-admin SHALL
change a submission's status. On acceptance with a schedule, the service SHALL create a
standard `Event` through the existing event path, whose visibility mirrors the
accepted visibility; declined and withdrawn submissions SHALL never be published.
Status transitions by an admin SHALL be recorded in the audit log.

#### Scenario: A member cannot self-publish

- **WHEN** a member sets requested visibility `public` and saves
- **THEN** the submission SHALL NOT appear on any public/marketing surface while
  its status is `submitted` or `under_review`

#### Scenario: Acceptance with a schedule creates an event

- **WHEN** an admin accepts a submission and supplies a schedule
- **THEN** a standard `Event` SHALL be created via the existing event path, with
  visibility matching the accepted submission, and the decision SHALL be audited

#### Scenario: A non-admin cannot change status

- **WHEN** a member attempts to set a submission's status to `accepted`
- **THEN** the attempt SHALL be denied and the status SHALL be unchanged

### Requirement: Member-supplied submission content is escaped in every rendered view

Member-supplied submission fields SHALL be HTML-escaped in every rendered view —
the member's own views AND the reviewer/admin views — so that no member-controlled
value can inject active markup into a viewer's page. Submission fields SHALL NOT
be rendered through any raw/unescaped (`|safe`) path. This protects the
higher-privileged reviewer specifically: a submission is untrusted author input
consumed in the admin's authenticated origin.

#### Scenario: A script-bearing title is inert in the admin view

- **WHEN** a member submits a title of `<script>alert(document.cookie)</script>`
  and an admin opens the review queue and the submission's detail
- **THEN** the title SHALL render as inert, escaped text and no script SHALL
  execute in the admin's session

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

### Requirement: Submission writes are CSRF-protected and rate-bounded

All state-changing submission requests SHALL be subject to the existing CSRF
protection for browser-facing POSTs — create, edit, withdraw, and admin
accept/decline. The number of open (non-terminal) submissions a single member
may hold SHALL be capped to bound spam and storage exhaustion; a create that
would exceed the cap SHALL be refused.

#### Scenario: A request without a valid CSRF token is rejected

- **WHEN** a submission create/edit/withdraw or admin decision POST arrives
  without a valid CSRF token
- **THEN** the request SHALL be rejected and no state SHALL change

#### Scenario: Exceeding the open-submission cap is refused

- **WHEN** a member who already holds the maximum number of open submissions
  attempts to create another
- **THEN** the create SHALL be refused with a clear error

