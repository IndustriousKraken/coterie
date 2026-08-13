# Change: The public event projection's field list admits description_html

## Why

Canon currently contradicts itself about a field that already ships.

`announcement-markdown`'s "Public event output carries server-rendered sanitized
HTML" states that `GET /public/events` **SHALL** include a server-rendered
sanitized rendering of the event description alongside the raw one.

`public-content-feeds`'s "Members-only events appear in /public/events with
sanitized fields" states that the projection **SHALL expose only** an enumerated
set of fields — `id`, `title`, `description`, `event_type`, `visibility`,
`start_time`, `end_time`, `timezone`, `location`, `image_url`, `max_attendees`,
`rsvp_required`, `registration_url`, `guest_price_cents`. `description_html` is
not among them.

One requirement mandates the field; the other forbids it by omission from a
closed list. Both are canon.

The code is not in doubt: `PublicEvent` carries `description_html`
(`src/api/handlers/public/mod.rs`), populated from the projected description
through the shared renderer, covered by a dedicated test file, and reflected in
the generated OpenAPI schema because the struct derives `ToSchema` and is
registered in `ApiDoc`. The shipped behavior is correct and is what both the
implementing change and its tests intended.

How it got here is worth recording, because the mechanism will recur. The
`a57-event-description-markdown` draft was flagged by the `[canon]` gate for
exactly this contradiction. The fix — amending this field list — was written
against `openspec/changes/a57-event-description-markdown/`, but by then a57 had
already been implemented and archived from the pre-fix draft. The correction
landed in a directory that upstream no longer had, so it never reached canon,
while the requirement it was meant to accompany did. The gate caught the defect;
the repair simply missed its window.

## What Changes

- `description_html` joins the enumerated field list in the public event
  projection requirement, so the closed list matches what the endpoint returns
  and what the other requirement already mandates.
- The field's meaning and its derivation rule are **referenced**, not restated.
  `announcement-markdown` already specifies that it is produced by the shared
  Markdown pipeline and that it is derived from the projected description rather
  than the underlying row — the rule that stops rendering from becoming a second
  path around members-only sanitization. Repeating that here would create two
  statements of one rule that can drift.

## What this does not do

- **No behavior change.** Nothing in the API, the renderer, the templates, or the
  tests changes. This is canon catching up to code that already satisfies both
  the intent and the tests.
- **It does not touch the `announcement-markdown` requirement.** That requirement
  is correct as written; it only needed the other side to stop contradicting it.
- **It does not revisit the OpenAPI clause.** There is no OpenAPI document in the
  repository because the schema is generated from the Rust types by `utoipa`;
  `PublicEvent` derives `ToSchema` and is registered in `ApiDoc`, so the field is
  already reflected and the clause is satisfied.
