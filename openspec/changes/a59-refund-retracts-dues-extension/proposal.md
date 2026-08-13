# Change: Refunding membership dues retracts the dues it granted

## Why

`handle_charge_refunded` treats one payment kind differently from the other two,
and nothing says it should.

When a refund is observed, the handler flips the payment to `Refunded` and then:

- **event fee** → releases the seat. The code comment states the principle:
  *"a member doesn't keep a confirmed seat for an event whose fee went back to
  them."*
- **class pass** → cancels the enrollment and the attendance rows for sessions
  that have not started.
- **membership dues** → **nothing.**

So `dues_paid_until` and `dues_extended_at` survive a refund untouched. The
member keeps the membership they were refunded for, stays Active, and every
downstream system that reads the dues window keeps treating them as paid.

This is not theoretical. On the production instance, member `1+newnewtest` was
charged $80.00 on 2026-07-11 and **refunded**; the payment row still carries its
`dues_extended_at`. On 2026-08-11 the renewal machinery charged the same member
$80.00 again, because the dues window the refund left in place said they were a
paying member due for renewal. A refund produced a renewal.

The recurring-billing runner is behaving correctly given its inputs — it
*"identifies members whose `dues_paid_until` is approaching"* and charges them.
The defect is upstream: the refund never moved `dues_paid_until`, so the runner
was reading a window that the organization had already given the money back for.

Framing this as an auto-renew problem would fix the symptom and leave the cause.
The stale dues window is also what keeps the member Active, what the portal shows
them, and what any future consumer of "is this member paid up" will read.

## What Changes

- A refunded membership payment retracts the dues extension it granted, bringing
  the third payment kind in line with the two that already do this. The rule
  becomes uniform: **a refund undoes what its payment bought.**
- The retraction is by the amount that payment extended, not a reset to a fixed
  date. A member may hold dues from several payments, and only the refunded one
  is being undone.
- It applies on both refund paths — the admin refund route and the out-of-band
  `charge.refunded` webhook — exactly as the event-seat rule already does. A
  refund issued from the Stripe dashboard must have the same effect as one issued
  from the admin UI.
- A payment that never extended dues retracts nothing, so the operation is safe
  to apply uniformly and safe to repeat.
- The retraction is recorded in the audit log the way the event and series
  refunds already record theirs, so the change in a member's dues window has a
  traceable cause.

## Partial refunds

Partial refunds are deliberately excluded and remain as they are: the handler
leaves the row untouched and raises an `AdminAlert`, on the stated grounds that
partial refunds *"muddle dues / campaign accounting."*

That reasoning holds — a half-refunded year is not obviously half a year of
membership — but it leaves a partially-refunded membership with the same stale
window and no automatic correction at all. This change does not resolve that. It
makes the alert say so explicitly, so an operator handling one knows the dues
window is theirs to adjust rather than assuming the system handled it.

## What this does not do

- **It does not change auto-renew, the renewal runner, or scheduled payments.**
  They read the dues window; this change makes the window truthful.
- **It does not cancel a membership or change a member's status directly.**
  Retracting the extension may move a member to Expired at the natural
  transition, through the existing path, rather than by a new status write.
- **It does not touch partial refunds' accounting**, per above.
- **It does not retroactively repair existing rows.** Members already carrying a
  refunded-but-unretracted window need an operator decision, not a migration
  guessing at intent.
