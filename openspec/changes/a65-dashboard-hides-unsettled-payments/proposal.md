# Change: Every member-facing payment list hides unsettled payments, not just one

## Why

`IndustriousKraken/coterie#120` asked for abandoned-checkout rows to stop
appearing in a member's payment history. The fix landed for the Payments page in
v1.0.16. The reporter's follow-up comment is precise about what it missed:

> The Recent Payments are still visible on the Dashboard but are no-longer
> present after clicking "View all" or manually navigating to "Payments".

That is still true, and the reason is visible in the code. There is a shared
predicate, `partials::is_member_visible`, admitting `Completed` and `Refunded`.
The payments list fragment calls it. The dashboard's `recent_payments` does not —
its comment says outright that it returns the five most recent payments *"for
this member, regardless of status."*

So a member who abandons a Stripe checkout sees the resulting `Failed` row on the
dashboard, clicks through to Payments, and finds it gone. Two surfaces in the
same portal disagree about what the member's payment history contains.

**The code is not in violation of its spec — the spec is under-scoped.**
`payment-history-and-receipts` says the rule *"applies to `GET /portal/payments`
and its HTMX list fragment."* It names one surface and never mentions the
dashboard fragment. An implementation that filtered exactly there is a faithful
reading. This needs a requirement that describes the member's view rather than a
route.

## What Changes

- The rule becomes a property of every member-facing surface that lists payments,
  rather than of one page. The dashboard fragment is brought under it, and so is
  anything added later.
- Filtering happens **before** any limit is applied. This is the part most likely
  to be got wrong: the dashboard truncates to five and then renders. Filtering
  after truncation would show a member with five abandoned checkouts an empty
  dashboard while they have completed payments sitting just outside the window —
  a new bug wearing the old one's fix.
- One predicate stays the single home for the decision. The defect being fixed is
  a second surface not using it, so the answer is not a second copy of it.

## Why this matters beyond tidiness

A member reading their own payment history is checking whether they were charged.
Two surfaces giving different answers to that question is worse than either
answer alone, because it teaches them that the portal's numbers cannot be
trusted — and the surface showing the extra rows is the one showing scary-looking
`Failed` entries for money that was never taken.

## What this does not do

- **It does not change what "settled" means.** `Completed` and `Refunded` remain
  the member-visible set.
- **It does not touch admin views.** An admin continues to see every status,
  including `Pending` and `Failed`; that is how an abandoned checkout gets
  diagnosed at all.
- **It does not change receipts or the dues statement**, which already reflect
  settled payments only.
- **It does not delete or hide the underlying rows.** They remain in the database
  and in the admin surface; this is about what a member is shown.
