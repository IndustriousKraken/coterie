# Design — a41-paid-events-member-registration

## The one invariant

**Never hold money for a seat that does not exist, and never hold a seat that
nobody paid for.** Every decision below falls out of that.

## Seat state machine

```
                 free event
  (none) ─────────────────────────────────► Registered
     │
     │ paid event: claim seat, then create Checkout
     ▼
 PendingPayment ──checkout.session.completed──► Registered
     │                                              │
     │ checkout.session.expired                     │ charge.refunded
     │ payment_intent.payment_failed                │ / admin refund
     │ member abandons                              ▼
     ▼                                          Cancelled
 seat released (payment → Failed)
```

`PendingPayment` is the only new state. `Registered`, `Cancelled` and
`Waitlisted` keep their meanings; `Waitlisted` stays unused (no waitlist in v1).

## Ordering: claim the seat before creating the session

The registration path is:

1. **In one transaction**: count held seats, reject if full, insert the
   attendance row as `PendingPayment`.
2. Create the `Pending` payment row and the Stripe Checkout session.
3. If step 2 fails, delete the attendance row (release the seat) and return the
   error.

Claiming first means a full event can never mint a Checkout session. The reverse
order — session first, seat second — has a window where two members both pay for
the last seat, and the loser has to be refunded. Refunds are a worse failure
mode than a rejected registration, so the ordering is not negotiable.

## Seats are released by payment status, not by row deletion

The held-seat count is:

```sql
SELECT COUNT(*) FROM event_attendance a
LEFT JOIN payments p ON p.id = a.payment_id
WHERE a.event_id = ?
  AND ( a.status = 'Registered'
     OR (a.status = 'PendingPayment' AND p.status = 'Pending') )
```

A `PendingPayment` row whose payment has gone `Failed` stops holding its seat
**without anything having to delete it**. That matters because the existing
`handle_expired_session` already flips an abandoned checkout to `Failed` — so
abandonment releases the seat through machinery that is already written and
already tested. Same for `payment_intent.payment_failed`.

The dead row is kept rather than deleted so the admin roster can show "started
paying, didn't finish" — useful when a member swears they paid.

**Known ceiling:** if a webhook never arrives at all, that seat stays held. The
Checkout session is created with a 60-minute expiry to bound it, and the admin
roster gets an explicit "release seat" control. A background sweeper is
deliberately not built — it is speculative until an org actually reports a stuck
seat, and the manual control covers it.

## Zero is stored as zero

`member_price_cents` is `NOT NULL DEFAULT 0`. The tempting alternative — nullable,
with `NULL` meaning free — was rejected because `NULL` already means something
else. It means "unknown" or "not entered", and a free event's price is neither: it
is known, and it is zero.

Using `NULL` as a sentinel for zero breaks the queries nobody thinks to test:

```sql
WHERE member_price_cents = 0       -- matches nothing; every free event missed
WHERE member_price_cents <= 2000   -- omits free events (NULL <= 2000 is unknown)
SELECT AVG(member_price_cents)     -- silently computed over paid events only
ORDER BY member_price_cents        -- free events sort somewhere implementation-defined
```

Each of those fails *silently* — a wrong answer, not an error — and each requires
a future reader to know an undocumented convention to get right. Backfilling
existing rows to `0` is safe because it asserts something already true of them.

This also keeps the operator-facing behavior honest: a blank field and a typed
`0` both store `0`, so nobody has to learn our storage convention to price an
event at nothing.

## Why `EventFee { event_id }` and not `Other`

`PaymentKind::Other`'s doc comment already says "merch, event fees", so `Other`
is the tempting zero-migration answer. It is wrong here for three reasons:

1. **The refund path needs to find the seat.** `charge.refunded` arrives with a
   payment id and nothing else. Without `event_id` on the kind there is no way
   to release the right seat without a side table.
2. **`Other` has no side effects by contract.** Event fees do (they confirm a
   seat). Overloading `Other` would make "no automatic side-effects" false.
3. **Revenue reporting.** The billing dashboard splits dues vs donations; event
   revenue silently landing in an untyped bucket is a reporting bug waiting to
   be filed.

The DB string is `"event_fee"`. Existing rows are unaffected — the mapping is
additive, and `"other"` keeps deserializing to `Other`.

## Double-charge prevention

Before claiming a seat the service checks for an existing non-`Failed` event-fee
payment by this member for this event:

- a `Completed` one → already paid; return the existing registration, charge
  nothing.
- a `Pending` one → a checkout is already in flight; return that session's URL
  rather than minting a second one.

This makes the register button idempotent under double-click and back-button,
which is the realistic way a member gets charged twice.

## Deleting a paid event: refund first, delete second

`event_attendance` is `ON DELETE CASCADE` from `events`. So deleting a paid event
would silently vaporize the roster while the charges stand — the worst possible
outcome. The delete handler therefore refunds every `Completed` event-fee payment
first, and **aborts the delete if any refund fails**, surfacing the error. An
event that cannot be fully refunded stays alive, visible, and fixable, rather
than becoming an invisible pile of unrefunded charges.

Cancelling a *series* occurrence is out of scope here (single events only); the
series change inherits this rule.

## What the member sees

The RSVP button on a paid event reads "Register — $30" instead of "RSVP". Free
events are visually and behaviorally identical to today. After paying, Stripe
returns the member to the event page, which shows their confirmed seat; the
Checkout `success_url` carries no trust — the seat is confirmed by the webhook,
never by the redirect.

## Rejected alternatives

- **Charge on arrival / invoice later.** Needs a collections story for people who
  don't pay. The whole point is to take money up front.
- **A separate `event_registrations` table.** `event_attendance` already has the
  right key, the reminder integration, and the roster queries. A parallel table
  would double the read paths for one nullable column and one status.
- **Reusing the donation flow with a note field.** Untyped money, no seat, no
  capacity, no refund linkage. It is the thing orgs do today in Venmo, and the
  reason this change exists.
