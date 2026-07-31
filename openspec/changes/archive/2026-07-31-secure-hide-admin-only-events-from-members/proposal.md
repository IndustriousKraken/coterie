# Hide `AdminOnly` events from the member portal

## Why

`EventVisibility::AdminOnly` is a real, admin-settable level. The admin
forms offer it (`templates/admin/event_new.html:64`,
`templates/admin/event_detail.html:109`), Discord routes those events to
the admin-alerts channel with an `**[Admin only]**` prefix
(`src/integrations/discord.rs:339,346`), `EventPublished` is deliberately
not dispatched for them (`src/service/event_admin_service.rs:198`), and
the public surfaces exclude them (`openspec/specs/paid-events/spec.md:423`
— "A `MembersOnly` or `AdminOnly` event SHALL NOT become readable or
registerable").

The **member** portal excludes nothing. Every Active/Honorary member sees
`AdminOnly` events in full, and can register for them:

- `src/web/portal/events.rs:67-71` — `events_list_api` calls
  `event_repo.list(200, 0)` or `event_repo.list_upcoming(50)` and filters
  only by `event_type`; `AdminOnly` rows are rendered with title,
  description, location, image, and price.
  (`src/repository/event_repository.rs:472,494` — neither method filters
  on `visibility`.)
- `src/web/portal/dashboard.rs:143-146` — `upcoming_events` does the same
  on the dashboard. Its comment claims "visibility filtering is per-event
  inside the template, not at the repo layer" — no template does any such
  filtering; the fragment is built inline at `dashboard.rs:183-219`.
- `src/web/portal/events.rs:419-439` — `rsvp_event` loads the event by id
  and passes it straight to `EventRegistrationService::register` with no
  visibility test, so `POST /portal/api/events/:id/rsvp` seats any member
  on an `AdminOnly` event. `cancel_rsvp_event`
  (`src/web/portal/events.rs:549`) has the same gap.

Harm: a broken authorization control leaking exactly the content the level
exists to protect. An `AdminOnly` event ("Board meeting — membership
revocation for …", plus its description and location) is disclosed to the
entire membership, and a member can write themselves onto its roster.

**This is a contract change**, which is why it is a spec-lane change
rather than an issue. Canon's `member-content` requirement is titled
"Members see all events and announcements (public + members-only)" and its
body says only "Members SHALL see both public and members-only content" —
it never states what happens to `AdminOnly`, and the "all events" framing
reads as permitting today's behavior. Fixing this narrows what two
documented member endpoints return and adds a new refusal to a documented
member endpoint, so the requirement itself is corrected here.

## What Changes

- Add one home for the rule next to the existing `publicly_registerable`
  in `src/domain/event.rs`: `Event::visible_to_member(&self, member) ->
  bool`, true when the event is not `AdminOnly`, or when the member is an
  admin. Admins keep seeing everything, on the member surfaces as well as
  in `/portal/admin/events`.
- Apply it in the four member-surface call sites: `events_list_api`,
  `upcoming_events`, `rsvp_event`, `cancel_rsvp_event`.
- An RSVP/cancel against an event the member cannot see answers with the
  existing "Event not found" fragment — the same non-disclosing shape the
  public registration path already uses, so a member cannot probe for
  admin-only event ids.
- The repository methods (`list`, `list_upcoming`) are left alone: `list`
  is also used by the admin surfaces
  (`src/web/portal/admin/events/single.rs:117`,
  `src/web/portal/admin/events/occurrences.rs:399`), which must keep
  seeing `AdminOnly` rows.

## Impact

- `src/domain/event.rs` — new `Event::visible_to_member`.
- `src/web/portal/events.rs` — `events_list_api`, `rsvp_event`,
  `cancel_rsvp_event`.
- `src/web/portal/dashboard.rs` — `upcoming_events` (and its stale
  comment).
- Spec delta: `openspec/specs/member-content/spec.md` — both requirements
  modified.

Operator follow-up: an org that has been using `AdminOnly` as a
scheduling-only marker may find those events disappear from members'
event lists after this change; that is the intended level semantics. Any
member RSVP rows already written against an `AdminOnly` event survive and
remain visible on the admin roster — they are not swept, because deleting
attendance history is not this change's business.
