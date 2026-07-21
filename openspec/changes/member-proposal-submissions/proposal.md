# member-proposal-submissions

## Why

Members have no way to submit content for the organization to review — talk and
presentation proposals being the motivating case (a hackerspace or professional
group running a "call for sessions"), but the same shape fits workshop pitches,
project showcases, and poster sessions. Orgs that don't want it should never see
it, so the capability is **off by default** behind a settings toggle; enabling
it does not change any existing behavior.

This is deliberately a **bounded domain feature, not a form-builder.** A generic
form engine (arbitrary field types, per-field validation, EAV storage, generic
rendering) is a product unto itself and a large, poorly-bounded security surface;
generality here comes from the toggle, not from arbitrary configurability. A
fixed, sensible submission schema covers the 80% case, and orgs needing extra
attributes can lean on the existing `member-custom-fields` pattern rather than a
builder.

Because this adds net-new behavior with its own persisted state, it is a
**change** (new `submissions` capability), not an issue.

### Security framing (why this change carries security requirements)

This is the first surface where an **authenticated member authors content —
including an uploaded file — that a higher-privileged admin/reviewer later
opens.** That privilege-crossing data flow is the whole risk: a submitter is a
low-trust author and a reviewer is a high-value target. The dangerous paths are
therefore first-class requirements below, not afterthoughts:

- **Stored XSS into an admin session.** Member-supplied title/abstract rendered
  unescaped in the reviewer's authenticated portal view would run script in the
  admin's origin — session/CSRF-token theft, account takeover. (Exactly the
  "swipe the first admin who reads a proposal" attack.)
- **Malicious / mis-typed uploads.** Polyglot files, executable content served
  inline, path traversal via the uploader's filename, oversized/zip-bomb inputs.
- **Broken access control (IDOR).** Reading another member's private proposal or
  fetching a private attachment by guessing its URL.
- **Unauthorized publication.** A member pushing content to the public marketing
  surface without a reviewer's decision.

## What Changes

- A new `submissions` capability, gated by a boolean org setting
  `submissions.enabled` (default **false**). When disabled, no submission routes
  are mounted and no portal UI is shown.
- **Member surface:** an authenticated member can create a submission (title,
  abstract, requested visibility public|members, optional PDF attachment,
  optional preferred date/duration), and can view/edit/withdraw **only their
  own** submissions while they are still `submitted`.
- **Reviewer surface:** an admin sees a review queue and can move a submission
  through `submitted → under_review → accepted | declined`; withdrawing is the
  member's equivalent terminal state. Reviewer decisions are audited.
- **Publication is reviewer-gated.** A member's "open to public" is a *request*.
  Nothing reaches the public/marketing surface until an admin **accepts** it; on
  acceptance with a schedule, the service creates a standard `Event` via the
  existing event path (reusing calendar, RSVP, timezone, and the public feed
  rather than duplicating them). `visibility` maps onto the existing event
  Public/MembersOnly concept.
- **Attachments** reuse `src/web/uploads.rs::save_uploaded_file` (generated
  filename, size cap, authoritative magic-byte sniff). For the MVP the only
  accepted type is **PDF** (`%PDF-`, cleanly detectable); PPTX/other ZIP-based
  Office formats are deferred precisely because they share ZIP magic with
  arbitrary/zip-bomb archives and cannot be safely distinguished by sniffing.
  Private attachments are served through a **new authorization-gated route**,
  NOT the existing public `/uploads/:filename`, and always as
  `Content-Disposition: attachment` (never rendered inline).
- All member-supplied text is HTML-escaped in every rendered view (Askama
  auto-escaping; no `|safe` on submission fields), with CSP as defense in depth.
- State-changing requests go through the existing CSRF layer; field lengths are
  bounded; a per-member cap limits open submissions.

## Impact

- **Spec:** new capability `submissions` — 6 ADDED requirements (feature +
  security invariants). No existing capability is modified.
- **Code (new):** `src/domain/submission.rs`, `src/repository/submission_repository.rs`,
  `src/service/submission_service/` (validation + state machine + promote-to-event),
  `src/web/portal/submissions.rs` (member), `src/web/portal/admin/submissions/`
  (review queue + gated attachment download), a migration for the `submissions`
  table and the `submissions.enabled` setting.
- **Code (extend):** `src/web/uploads.rs` — add PDF magic-byte detection and a
  document-aware save path (or a `detect_document_format`), keeping the sniff
  authoritative; SVG stays disallowed.
- **Reuse:** existing event creation (promotion), settings service (toggle),
  CSRF layer, audit log, field-bounding helpers.
- **Tests:** unit + integration, including security regressions —
  admin-view escaping of a `<script>` title, cross-member read returns 403/404,
  private-attachment fetch by a non-owner is denied, non-PDF / oversized upload
  rejected, CSRF-less POST rejected, member cannot self-publish.
- **Behavior for orgs that don't enable it:** none — default-off.
- **Deferred (v2):** multi-presenter, member voting/ranking, comment threads,
  scheduling-conflict detection, non-PDF attachment types.
