# Tasks

## 1. Shared pipeline

- [ ] 1.1 Reuse `crate::util::markdown::render_announcement_markdown` for event
  descriptions. Do not add a second renderer, a second ammonia configuration, or
  a second safe-subset list — the whole point of the existing requirement is that
  there is one.
- [ ] 1.2 If the function's name now reads as announcement-specific, rename it to
  match what it renders. A rename is cheap; two renderers are not.

## 2. Public output

- [ ] 2.1 Add a rendered description field to the `PublicEvent` projection in
  `src/api/handlers/public/mod.rs`, alongside the raw `description`, mirroring how
  `content_html` sits beside `content` for announcements.
- [ ] 2.2 Render from the **projected** value, not the underlying row. The
  members-only sanitizer replaces `description` with a fixed placeholder before
  the projection is built; rendering the row's real description would route around
  that and publish what the projection withheld. Order this after sanitization and
  assert the ordering in a test rather than relying on it staying that way.
- [ ] 2.3 Update the OpenAPI schema for the endpoint, as the announcement field
  already is.

## 3. Coterie's own surfaces

- [ ] 3.1 `templates/events/register.html` renders `description` as text inside a
  `whitespace-pre-line` block. Render the sanitized HTML instead.
- [ ] 3.2 Same for the class registration page and any member-portal surface
  showing an event description.
- [ ] 3.3 The rendered HTML is already server-sanitized by the shared pipeline —
  mark it as safe at the template boundary exactly the way the announcement
  templates do, and nowhere else. Do not mark the raw description safe anywhere.

## 4. Editor hint

- [ ] 4.1 Add the Markdown hint to the event description field on the create form
  and the edit form, copying the announcement editor's wording from
  `templates/admin/announcement_detail.html` rather than composing new text.
- [ ] 4.2 Include the placeholder text treatment the announcement editor uses on
  its create form, so the two behave the same before anything is typed.

## 5. Tests

- [ ] 5.1 An event description containing bold, a list, and an `https` link
  renders to the safe HTML equivalents.
- [ ] 5.2 An event description containing `<script>`, an `<img>`, an `onclick`,
  and a `javascript:` URL renders none of them.
- [ ] 5.3 The rendered field for a members-only event derives from the placeholder
  and contains no fragment of the real description. This is the ordering guard for
  2.2 — write it so it fails if rendering is ever moved ahead of sanitization.
- [ ] 5.4 The raw description is unchanged by rendering — the stored value still
  round-trips to the admin edit form exactly as typed.
- [ ] 5.5 Registration and class pages render emphasis as formatting, not as
  literal asterisks.
- [ ] 5.6 Both event forms carry the Markdown hint, and its wording matches the
  announcement editor's.
- [ ] 5.7 Assert only one Markdown renderer exists in the codebase — a grep-style
  check. The defect class is a second pipeline whose safe subset drifts from the
  first.
