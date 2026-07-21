# Tasks

New capability, off by default. Every task that touches an untrusted field or
the attachment path is a security control — call those out in review.

## 1. Storage & toggle

- [ ] 1.1 Migration: create the `submissions` table (schema per `design.md`),
  with FKs to `members` and a nullable `event_id` FK to `events`; index
  `submitter_member_id` and `status`.
- [ ] 1.2 Register the `submissions.enabled` boolean setting (category
  `submissions`, default `false`) through the settings service so writes are
  audited; expose it on the admin settings page.

## 2. Domain, repository, service

- [ ] 2.1 `src/domain/submission.rs`: the `Submission` struct and a
  `SubmissionStatus` enum (`submitted`/`under_review`/`accepted`/`declined`/
  `withdrawn`/`scheduled`) with canonical wire strings.
- [ ] 2.2 `src/repository/submission_repository.rs`: CRUD + `list_for_member`,
  `list_for_review`, `count_open_for_member`. Reads used for authorization MUST
  return the `submitter_member_id` so the caller can check ownership.
- [ ] 2.3 `src/service/submission_service/`: create (bounds + open-submission cap
  + attachment handling), owner-scoped edit/withdraw, admin status transitions
  (validate the allowed transition graph), and `promote` (create an `Event` via
  the existing event path and set `event_id`). `submitter_member_id` and
  `decided_by` are ALWAYS taken from the authenticated principal, never the body.

## 3. Upload handling (extend, keep sniff authoritative)

- [ ] 3.1 `src/web/uploads.rs`: add PDF magic-byte detection (`%PDF-`) and a
  document save path that accepts ONLY sniffed PDF, reuses the generated-name +
  size-cap logic, and keeps SVG/other types rejected. The client extension /
  content-type remain hints, never the decision.
- [ ] 3.2 Add a gated attachment download route (member portal) that loads the
  owning submission, authorizes (submitter OR reviewer OR accepted+public), and
  streams with `Content-Disposition: attachment` and `X-Content-Type-Options:
  nosniff`. Do NOT serve private attachments via `/uploads/:filename`.

## 4. Web surfaces

- [ ] 4.1 Member: `src/web/portal/submissions.rs` — list own, create form,
  edit/withdraw (owner + `submitted`-only for edit). CSRF tokens on all forms.
- [ ] 4.2 Admin: `src/web/portal/admin/submissions/` — review queue, detail,
  accept (with optional schedule → promote) / decline, reviewer note. CSRF on
  all decisions; audit each transition.
- [ ] 4.3 Templates: render `title`/`abstract`/`reviewer_note` through Askama
  auto-escaping ONLY — no `|safe` on any submission field, in member OR admin
  templates. Keep pages under the existing CSP.
- [ ] 4.4 Mount all submission routes behind the `submissions.enabled` check so
  they do not exist when the toggle is off.

## 5. Tests (feature + security regressions)

- [ ] 5.1 Create → row persisted with status `submitted` and submitter from the
  session; oversized title/abstract rejected.
- [ ] 5.2 **IDOR:** member A reading/editing member B's submission is denied
  (404/403) and discloses nothing; owner succeeds.
- [ ] 5.3 **Stored XSS:** a submission titled `<script>…</script>` rendered in
  the admin detail view appears escaped/inert (assert the raw `<script>` token
  is not present as markup) — mirror the encoding assertions in the marketing
  site's `main.test.js`.
- [ ] 5.4 **Attachment authz:** a non-owner, non-reviewer fetching a private
  attachment is denied; an accepted+public attachment is reachable; the response
  carries `Content-Disposition: attachment`.
- [ ] 5.5 **Upload type:** a non-PDF (e.g. a PNG, and a `PK\x03\x04` zip renamed
  `.pdf`) is rejected by the sniff; an oversized PDF is rejected.
- [ ] 5.6 **Publication gate:** a member's `public` submission is absent from the
  public surface until an admin accepts; acceptance-with-schedule creates an
  `Event` with matching visibility; a member cannot set status `accepted`.
- [ ] 5.7 **CSRF:** a create/withdraw/decision POST without a valid token is
  rejected. **Cap:** creating past the open-submission cap is refused.
- [ ] 5.8 Toggle off → routes not mounted (request returns not-found) and no
  portal entry point.

## 6. Verify

- [ ] 6.1 `openspec validate member-proposal-submissions --strict` passes.
- [ ] 6.2 `cargo test` (new + existing suites) green; `cargo clippy` clean on
  touched files.
