# import-stripe-payment-history

## Why

The MemberPress migration brought over members, live subscriptions, and
paid-through dates, but **none of their payment history and none of their
saved cards**. Two concrete gaps:

- **No payment history / receipts.** Members write these dues off on their
  taxes, so they need a record of what they paid. Today Coterie's `payments`
  table has nothing for any charge that happened before (or outside) Coterie,
  so there is no receipt and no year-end total to hand an accountant.
- **No saved cards.** Coterie shows only cards in its own `payment_methods`
  table, and the import populated zero. So a member on a Stripe-managed
  subscription sees no card at all, and even the card that is actively
  billing them is invisible in Coterie.

Both are the same shape of gap — data that already lives in Stripe was never
backfilled into Coterie — so this change backfills both and adds the
member-facing artifacts (receipts + an annual statement).

## What Changes

- **Backfill payment history from Stripe.** For each member with a Stripe
  customer, import their historical charges/invoices as `payments` rows,
  idempotently (keyed on the Stripe id, so re-running never double-imports).
- **Backfill saved cards from Stripe.** For each member's Stripe customer,
  hydrate `payment_methods` from Stripe's attached cards, de-duplicated by
  card fingerprint so a card that already exists is not added twice.
- **Per-payment receipts.** A member can view and download a receipt for any
  recorded payment, rendered from the org receipt settings Coterie already
  has.
- **Annual dues statement.** A member (and an admin, per member) can pull a
  per-calendar-year summary of dues paid — the artifact people actually hand
  to an accountant for a write-off.
- **Email a receipt on each charge, going forward.** When a payment is
  recorded (subscription invoice via webhook, or a Coterie-initiated charge),
  email the member a receipt — **gated on an email provider being
  configured**. This depends on the operator wiring an email service
  (SMTP / SendGrid / etc.) in Coterie's email settings first; when none is
  configured the receipt is still viewable in-portal and no email is
  attempted.

## Impact

- **Spec:** new capability `payment-history-and-receipts`.
- **Code:** a backfill routine (migration or guarded one-shot admin action)
  that pages each Stripe customer's charges and cards into `payments` /
  `payment_methods`; reuse the existing per-Stripe-id uniqueness on `payments`
  for idempotency and card fingerprint for card de-dup; receipt + annual
  statement views under the member portal (and an admin per-member view);
  a receipt-email hook on payment recording that no-ops when email is
  unconfigured.
- **Prerequisite (migration step):** configure an email provider in Coterie's
  email settings before enabling receipt emails. Backfill, receipts, and the
  annual statement do not need email and can ship first.
- **Testing:** the backfill and receipt/statement logic are unit-testable via
  the existing `FakeStripeGateway` seam (feature `test-utils`) — no real
  Stripe keys required; the fake returns canned charges/cards.
- **Out of scope:** re-issuing historical receipts by email (only newly
  recorded charges trigger an email); tax-authority-specific formatting.
