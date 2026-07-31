# Refuse member class enrollment on an `AdminOnly` class

## Why

The `secure-hide-admin-only-events-from-members` change closed the
`AdminOnly` hole on four member surfaces — `events_list_api`,
`upcoming_events`, `rsvp_event`, `cancel_rsvp_event` — by adding
`Event::visible_to_member` (`src/domain/event.rs:95`). It did not touch the
fifth member-surface registration endpoint, the class-scope sibling of RSVP:

- `src/web/portal/events.rs:496-541` — `enroll_in_series`, backing
  `POST /portal/api/series/:id/enroll` (`src/web/portal/mod.rs:637`). It loads
  the series by id (`src/web/portal/events.rs:504`) and hands it straight to
  `SeriesEnrollmentService::enroll` (`src/web/portal/events.rs:520`) with no
  visibility test whatsoever.

An `EventSeries` row carries no `visibility` of its own — the occurrences do
(`src/domain/event.rs:177-205` has no visibility field), which is exactly why
the public class page resolves the rule against an occurrence
(`load_enrollable`, `src/web/templates/class_register.rs:153-173`:
"a series row carries no visibility of its own"). The member enroll path
performs no equivalent resolution.

Attacker: any authenticated non-admin Active/Honorary member who holds an
`AdminOnly` series id. Harm:

- `SeriesEnrollmentService::enroll` runs to completion, so
  `seat_future_occurrences` (`src/service/series_enrollment_service.rs:353`)
  writes an `event_attendance` row for the member on **every future
  `AdminOnly` occurrence** — the member writes themselves onto the roster of
  events the level exists to hide, which is precisely what canon already
  forbids for the single-event path ("no `event_attendance` row SHALL be
  created").
- On a priced class the same call mints a Stripe Checkout session whose line
  item is the class title, resolved by `class_title`
  (`src/service/series_enrollment_service.rs:437`) from the `AdminOnly`
  occurrence and passed to
  `create_series_pass_checkout_session`
  (`src/service/series_enrollment_service.rs:246-255`). The member then reads
  that title on Stripe's hosted page — a direct disclosure of `AdminOnly`
  content, and a `series_enrollment` + `payments` row to go with it.

The events list already refuses to render the enroll control for such a class
(`src/web/portal/events.rs:94` filters on `visible_to_member` before the
class-offer lookup at line 149), so the endpoint is reachable only by posting
the id directly — the same reachability the RSVP hole had, and canon required
the check there anyway, on the stated ground that a distinguishable refusal
lets a member confirm which admin-only ids exist.

**This is a contract change**, which is why it is a spec-lane change rather
than an issue. Canon's `member-content` requirement "Members can RSVP to
events" enumerates only `POST /portal/api/events/:id/rsvp` and
`POST /portal/api/events/:id/cancel`; `paid-events` → "Enrolling in a paid
class takes payment before enrollment is confirmed"
(`openspec/specs/paid-events/spec.md:741`) specifies the enroll ordering and
says nothing about visibility. Today's behavior is therefore permitted, and
the fix adds a new refusal to a documented member endpoint. The requirement
that already owns this rule is MODIFIED rather than a parallel one added, so
the `AdminOnly` member-surface rule keeps one home and one vocabulary.

## What Changes

- `enroll_in_series` resolves the series' visibility the way the public class
  page already does — against an occurrence — and refuses when the requesting
  member may not see it, reusing the existing `Event::visible_to_member`
  rather than introducing a second rule.
- The refusal answers with the existing "Class not found" fragment
  (`render_class_error`, `src/web/portal/events.rs:547`) — byte-identical to
  the response an unknown series id already produces
  (`src/web/portal/events.rs:507`), so a member cannot distinguish
  "admin-only class" from "no such class".
- No `series_enrollment` row, no `event_attendance` row, no payment, and no
  Checkout session is created on the refused path — the check runs before
  `enrollment_service.enroll` is called.
- Admins keep enrolling normally, on `AdminOnly` classes included, since
  `visible_to_member` already returns true for them.
- A series with no occurrences at all resolves to "not visible" and takes the
  same not-found answer, matching `load_enrollable`'s existing treatment of
  an occurrence-less series (`src/web/templates/class_register.rs:170`).

## Impact

- `src/web/portal/events.rs` — `enroll_in_series` gains the visibility check
  before the enroll call.
- Spec delta: `openspec/specs/member-content/spec.md` — the existing
  "Members can RSVP to events" requirement is MODIFIED to cover the
  class-enroll endpoint (all existing scenarios retained).
- No schema change, no new route, no new dependency. The public class page and
  `POST /public/series/:id/enroll` are untouched — they already resolve
  visibility through `publicly_enrollable`.

Operator follow-up: an org that has been using `AdminOnly` on a recurring
class while expecting members to enroll should move that series to
`MembersOnly`; enrollment rows already written by members against an
`AdminOnly` class survive and remain visible on the admin roster — they are
not swept, on the same grounds the prior change gave for RSVP rows.
