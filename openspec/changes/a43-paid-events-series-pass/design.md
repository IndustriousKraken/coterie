# Design — a43-paid-events-series-pass

## Enrollment materializes attendance; it does not become a second read path

The central choice: when someone buys a six-week class, what represents "they are
coming on week 3"?

**Chosen — enrollment writes `event_attendance` rows** for every future occurrence,
and the daily horizon roll-forward writes rows for occurrences materialized later
while the enrollment is active.

**Rejected — enrollment as the sole record**, with per-occurrence rosters computed
as a union of direct attendance plus series enrollments. That forks every existing
read path: the roster, the capacity count, the reminder query, the iCal feed, and
check-in would each become a UNION, permanently, and each would be a place where
someone later forgets the second branch. `a42` already rejected the same shape for
guests; the reasoning is identical and worth being consistent about.

The cost of the chosen design is row count (a 6-week class of 12 people is 72
attendance rows) and a roll-forward that has to know about enrollments. Both are
cheap and local. The benefit is that "who is coming to this occurrence" has
exactly one answer in exactly one table, which is what every existing feature
already reads.

Past occurrences are **not** back-filled on a late join — you cannot attend a
session that already happened, and a roster showing someone at a night they
weren't there would be a lie that check-in data then inherits.

## A pass requires a bounded series

`event_series.until_date` is nullable, where `NULL` legitimately means "roll
forward forever" — a genuine absence, not a sentinel for a value, so it stays as
it is.

But a flat pass price on an unbounded series is incoherent: it sells unlimited
future sessions for one payment. That is a subscription, and Coterie already has
recurring billing for subscriptions. So a pass price is permitted only when
`until_date IS NOT NULL`, and the admin form says so plainly rather than letting
an operator accidentally sell an infinite class for $120.

## Capacity is a series-level number

For a paid class, "twelve seats" means twelve people in the class — not twelve per
night. So a paid series carries its own capacity, enforced at enrollment time, and
pass-holders' attendance rows are **not** re-checked against each occurrence's
`max_attendees`. They already bought the seat; discovering on week 4 that the room
cap rejected them would be taking money for nothing, which is the exact failure
a41 exists to prevent.

An occurrence of a paid class can still be individually over-subscribed by direct
single-event registrations if the org also prices the occurrences — an org doing
both is opting into managing two numbers, and the deferred drop-in-pricing feature
is where that gets designed properly rather than guessed at here.

## Flat pricing is a policy, stated as one

No proration, in either direction:

- **Late join** pays full price. They get fewer sessions for the same money.
- **Cancellation** refunds in full, however many sessions have already happened.

This is deliberately generous on refunds and deliberately blunt on late joins,
because the alternative — per-session accounting, partial refund math, and a
"what is a session worth" rule — is a real feature with real edge cases (what
about a cancelled occurrence? a member who attended two of six?) and no org has
asked for it. Stating it as policy in the spec means a future contributor
implements the simple rule on purpose rather than assuming proration was an
oversight.

## Cancelling one occurrence is not a refund event

`a35-recurring-event-exceptions` lets an admin cancel a single occurrence. For a
paid class that is a holiday skip, not a partial cancellation of the product, and
it triggers no refund. Deleting the **whole series** does refund everyone, mirroring
a41's refund-before-delete rule for a single event — including its abort-on-failure
behavior, so a series whose refunds don't all succeed stays alive and fixable.

## Payment kind: a sibling variant, not a generalized target

`PaymentKind::SeriesPass { series_id }` sits next to `EventFee { event_id }`.

The tempting alternative was to generalize a41's variant into
`EventFee { target: Occurrence(id) | Series(id) }`. Rejected: it rewrites
just-shipped a41 matching code for no behavioral gain, and the two really are
different products — one seat at one night versus a pass to a course. Two
variants, each carrying the id it needs, keeps every match site obvious and lets
the compiler enforce totality when either is handled.
