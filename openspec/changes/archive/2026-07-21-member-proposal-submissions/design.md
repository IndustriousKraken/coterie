# Design notes — member-proposal-submissions

Implementation guidance, not contract. The binding contract is
`specs/submissions/spec.md`. This file records the data model and, mostly, the
security reasoning — this feature's defining risk is that a **low-trust member
authors content (and a file) that a high-value admin later opens**, so the
threat model drives the design.

## Data model

`submissions` table (all member-authored fields are untrusted input):

- `id` UUID PK
- `submitter_member_id` FK → members (set from the session, NEVER from the body)
- `title` TEXT (bounded), `abstract` TEXT (bounded)
- `visibility_requested` TEXT `public` | `members`
- `attachment_path` TEXT NULL (server-generated `uploads/…` relative path)
- `preferred_start` naive wall-clock NULL + `timezone` (follow the
  `event-timezone` convention — store local wall-clock + IANA zone, never a
  frozen instant), `duration_minutes` INT NULL
- `status` TEXT `submitted` | `under_review` | `accepted` | `declined` |
  `withdrawn` | `scheduled`
- `reviewer_note` TEXT NULL, `decided_by` FK NULL, `event_id` FK NULL (set on
  promotion), `created_at`, `updated_at`

Setting: `submissions.enabled` (bool, category `submissions`, default `false`),
via the existing settings service so the toggle is audited like any other.

## Threat model and decisions

### 1. Stored XSS into a reviewer's session (primary threat)

The reviewer opens attacker-authored `title`/`abstract` in their authenticated
origin. Unescaped render → script runs as the admin → session cookie / CSRF
token exfiltration → the "swipe the first admin" takeover.

- **Decision:** rely on Askama's default auto-escaping and **forbid `|safe`** on
  any submission field, in the member views AND the admin queue/detail. This is
  cheap and total for a server-rendered portal; the requirement pins it with a
  `<script>`-title scenario.
- **Defense in depth:** the `security-headers` capability's CSP already
  constrains inline script; keep submission pages under it. No user field is
  ever placed in a JS/attribute/`url()` sink server-side.

### 2. Malicious or mis-typed uploads

- **Type:** `uploads.rs` already treats the magic-byte sniff as authoritative
  (extension is a hint). Extend that with a PDF check (`%PDF-`). **MVP is
  PDF-only** — deliberately: PPTX/DOCX/XLSX are ZIP containers sharing
  `PK\x03\x04` with arbitrary archives and zip bombs, so they cannot be
  distinguished from a hostile zip by sniffing. Adding them safely needs
  container inspection + decompression-bomb limits, which is out of MVP scope.
  **SVG stays disallowed** (it is an active-content format).
- **Inline execution:** even a valid PDF can carry JS and some browsers render
  PDFs inline. Serve every attachment with `Content-Disposition: attachment`
  (+ `X-Content-Type-Options: nosniff`) so it downloads rather than executing in
  the origin.
- **Path traversal:** never derive the storage path from the uploader's
  filename — reuse the existing generated-name behavior (`uploads/<generated>`).
- **DoS:** keep the existing 10 MB cap; enforce it *before* buffering where the
  multipart layer allows. The per-member open-submission cap bounds total stored
  bytes per member.

### 3. Broken access control (IDOR) — the bounty's explicit target

Two objects are enumerable: the submission and its attachment.

- **Submission:** every read/edit/withdraw resolves the row and checks
  `submitter_member_id == session.member_id` (admins bypass for review). Deny
  without disclosure (prefer 404). Mirror the ownership-check pattern already
  used by `member-saved-cards` / `member-profile`.
- **Attachment:** a private attachment must NOT be served by the public
  `/uploads/:filename` route (that route is unauthenticated and its names could
  be enumerated). A **new gated download route** loads the owning submission,
  runs the same authorization (submitter OR reviewer, OR the submission is
  `accepted` + `public`), then streams the file. Attachment filenames should be
  high-entropy so a leaked path is not itself an authorization bypass, but
  entropy is defense in depth — the authorization check is the control.

### 4. Unauthorized publication / privilege escalation of content

`visibility_requested = public` is a *request*, not a publish. Only an admin
acceptance moves content to the public surface (and only then via a created
`Event`). This prevents a member unilaterally defacing/spamming the marketing
site or planting malicious links there. The status transition authority
(`submitted → …`) is admin-only; a member can only reach `withdrawn`.

### 5. CSRF and mass-write abuse

All state-changing POSTs are browser-facing → the existing `csrf-protection`
layer applies unchanged. A per-member cap on open (non-terminal) submissions
bounds row/upload spam; consider reusing `rate-limiting` for burst creates if
the cap proves insufficient (deferred unless needed).

### 6. Promotion reuse (not a new event surface)

Promotion calls the existing admin event-creation path so all the audited,
timezone-correct, RSVP/feed machinery is reused rather than reimplemented — no
second event-write surface to secure. `event_id` links back for traceability.

## Deferred (v2, explicitly out of scope)

Multi-presenter, member voting/ranking, comment threads, scheduling-conflict
detection, non-PDF attachment types (needs container inspection + bomb limits).
