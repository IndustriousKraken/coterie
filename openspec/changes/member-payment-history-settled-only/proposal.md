# member-payment-history-settled-only

## Why

Reported in GitHub issue #120. When a member starts a dues checkout and backs
out on the Stripe page, a `Pending` payment row appears in their Payments history
and is later marked `Failed`. Every abandoned attempt leaves such a row, so a
member's history fills with `Failed`/`Pending` entries for payments they chose to
cancel — pure noise. (It is the same `Failed`-row accumulation that made the
carding-bot accounts undeletable.)

Show the member only the payments that represent money that actually moved —
`Completed` and `Refunded` — and hide `Pending`/`Failed` from their view. This is
a change to what the member Payments page displays, so it needs a proposal.

Hiding `Failed` from the member's list does not hide a genuine failure signal: a
real failed auto-renew surfaces through the dues-status pill (expired/unpaid) and
the dues-reminder emails, not the history list — so the member still learns they
need to fix their card, without the abandoned-checkout noise.

## What Changes

- The member Payments history (`/portal/payments`, `member_payment_row_from` /
  its list handler) SHALL list only `Completed` and `Refunded` payments;
  `Pending` and `Failed` are omitted from the member view.
- **Admin views are unchanged** — admins still see all statuses (they need
  `Pending`/`Failed` for support and reconciliation).
- Receipts and the annual dues statement are unaffected (both already key off
  settled/`paid_at` payments).

## Impact

- **Spec:** `payment-history-and-receipts` — 1 ADDED requirement ("The member
  payment history lists only settled payments"). No existing requirement pins the
  member list's status set, so nothing is modified.
- **Code:** the member payment list handler / `member_payment_row_from`
  (`src/web/portal/partials.rs` + its caller in `src/web/portal/payments.rs`)
  filters to `Completed`/`Refunded` before rendering. The admin payment list
  (`admin/partials.rs`) is untouched.
- **Tests:** a member with a mix of statuses sees only Completed/Refunded; an
  admin viewing the same member still sees Pending/Failed; receipts/statement
  behavior unchanged.
- **Out of scope:** cleaning up the accumulated `Pending`/`Failed` rows in the DB
  (they're just hidden here); that's a separate maintenance concern.
