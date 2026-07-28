## 1. One home for the visibility rule

- [ ] 1.1 In `src/domain/event.rs`, add
  `pub fn visible_to_member(&self, member: &crate::domain::Member) -> bool`
  to the `impl Event` block, immediately after `publicly_registerable`.
  Body: `member.is_admin || self.visibility != EventVisibility::AdminOnly`.
  Doc-comment it in the same style as `publicly_registerable` — one home
  for the rule so the decision is not re-derived (and re-broken) per call
  site.
- [ ] 1.2 Add a unit test `admin_only_event_is_hidden_from_non_admin` in
  `src/domain/event.rs`'s test module asserting `visible_to_member` is
  `false` for an `AdminOnly` event with a non-admin member, and `true` for
  the same event with an admin member, and `true` for `Public` and
  `MembersOnly` events with a non-admin member.

## 2. Apply it on the member listing surfaces

- [ ] 2.1 In `src/web/portal/events.rs::events_list_api`, extend the
  existing `filtered_events` filter closure (around line 88) with
  `if !e.visible_to_member(&current_user.member) { return false; }` before
  the event-type test.
- [ ] 2.2 In `src/web/portal/dashboard.rs::upcoming_events`, filter the
  `list_upcoming(5)` result with `visible_to_member` before building
  `event_summaries`. Because the filter now removes rows, request more
  than 5 from the repository (e.g. `list_upcoming(25)`) and `take(5)`
  after filtering, so an admin-only event near the top does not shrink a
  member's dashboard list. Replace the stale comment at
  `src/web/portal/dashboard.rs:143-145` ("visibility filtering is
  per-event inside the template") — no template filters anything — with
  one describing the filter that now runs here.

## 3. Apply it on the RSVP surface

- [ ] 3.1 In `src/web/portal/events.rs::rsvp_event`, after loading the
  event, return the existing "Event not found" `render_rsvp_error`
  fragment when `!event.visible_to_member(&current_user.member)`. Do not
  use a distinct message or status — a member must not be able to tell an
  admin-only event id from a nonexistent one.
- [ ] 3.2 Apply the same guard in
  `src/web/portal/events.rs::cancel_rsvp_event`, matching whatever
  not-found fragment that handler already returns.

## 4. Regression tests

- [ ] 4.1 Add a test in `src/web/portal/events.rs`'s test module:
  seed a `Public` event and an `AdminOnly` event, GET
  `/portal/api/events/list` as a non-admin Active member, and assert the
  body contains the public event's title and does NOT contain the
  admin-only event's title. Repeat as an admin member and assert both
  titles are present.
- [ ] 4.2 Add a test in `src/web/portal/dashboard.rs`'s test module doing
  the same for `/portal/api/events/upcoming`.
- [ ] 4.3 Add a test `rsvp_to_admin_only_event_is_refused`: POST
  `/portal/api/events/:id/rsvp` for an `AdminOnly` event as a non-admin
  Active member, assert the response carries the "Event not found" text
  and that `event_attendance` has no row for that (event, member) pair.
