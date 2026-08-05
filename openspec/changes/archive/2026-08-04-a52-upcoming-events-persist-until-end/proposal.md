# Change: An event stays upcoming until it ends, not until it starts

## Why

Every "upcoming" filter in the system tests `start_utc() > now`. At 7:00:00 PM a
7 PM event stops being upcoming — while it is happening, and while people are
still walking in.

That is exactly when the listing matters most. A member running twenty minutes
late opens the portal to check the address and the event is gone from
`/portal/events` and off the dashboard. The public site is worse: the same
predicate governs `/public/events`, so the event vanishes from the marketing home
page and from every subscribed calendar client at the moment it begins. The only
way back to the details is to already know the direct URL — which is the one
thing someone hurrying does not have.

There are three separate implementations of the predicate today
(`EventRepository::list_upcoming`, `EventRepository::count_members_only_upcoming`,
and the default branch of the `/public/events` handler), so the rule is already
being re-derived per call site and can drift. Fixing it in one place and having
the others read that place is worth as much as the behavior change.

`Event::end_utc()` already exists and already resolves the stored wall-clock
against the event's IANA zone the same way `start_utc()` does, so the correct
instant is available at every one of those sites. Nothing new has to be computed.

## What Changes

- An event remains upcoming through its end, not its start. An event in progress
  is listed, sorted where its start time puts it — first.
- The predicate gets one home in the domain, alongside `start_utc`/`end_utc`, and
  the three call sites use it instead of writing `start_utc() > now` themselves.
- `end_time` is optional, and a missing end time means the end is **unknown** —
  not that the event ends the instant it begins. An event with no recorded end
  therefore remains upcoming for a defined grace period after its start. Keeping
  the start-based rule for that case would silently reproduce the bug for exactly
  the events most likely to lack an end time.
- The grace period is a named domain constant of two hours, not a setting and not
  a literal repeated at call sites. Two hours is what this organization's own
  events actually run (the recurring HTB and training nights are recorded 19:00 to
  21:00), so an event with no recorded end behaves like the ones that have one.
  The fix for a bad guess is to record the end time, which is why this is not
  worth a settings row to explain.

Non-goals, all deliberate:

- **Reminders stay start-based.** A reminder is about arriving on time; it has
  nothing to do with how long the listing persists.
- **`series_enrollment_service`'s "any occurrence still upcoming" check stays
  start-based.** It gates whether a class can still be bought, and buying a pass
  during the final session is not something this change should quietly enable.
- **Past events remain excluded.** The `from`/`to` range on `/public/events` is
  still the only way to see events that have ended; the marketing calendar's
  past-by-month view is untouched.
- **The occurrence cancel/override controls stay start-based.** `admin-events`
  canon fixes those at `start_time < now` because exceptions only apply to the
  future. Whether an occurrence in progress can still be cancelled is a separate
  question from what a listing shows, and this change does not answer it.
- **No "happening now" badge.** Making the event reachable is the fix; labeling it
  is a separate question about presentation.
