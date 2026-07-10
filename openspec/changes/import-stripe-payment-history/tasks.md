# Tasks

## 1. Backfill payment history from Stripe

- [ ] 1.1 Add a gateway method to page a customer's charges/invoices from
  Stripe (amount, currency, created, description, invoice/charge id, status).
- [ ] 1.2 A backfill routine (guarded one-shot admin action or migration)
  that, for each member with a `stripe_customer_id`, creates a `payments` row
  per historical charge. Key on the Stripe id and rely on the existing
  per-Stripe-id uniqueness so a re-run imports nothing new.
- [ ] 1.3 Map each imported charge to the right `Payer` / `PaymentKind`
  (membership dues vs donation) where the Stripe metadata allows; default to
  membership dues for subscription invoices.

## 2. Backfill saved cards from Stripe

- [ ] 2.1 For each member's Stripe customer, list attached cards and insert
  `payment_methods` rows (brand, last4, exp, `pm_…` id), skipping any whose
  card fingerprint already exists for that member so re-adds do not duplicate.
- [ ] 2.2 Set the member's default `payment_methods` row from the Stripe
  customer's default payment method when one is known.

## 3. Per-payment receipts

- [ ] 3.1 A member-facing receipt view/download for any of their `payments`
  rows, rendered from the org receipt settings.
- [ ] 3.2 An admin per-member view of the same, for support.

## 4. Annual dues statement

- [ ] 4.1 A per-member, per-calendar-year statement summing dues paid, with a
  printable/downloadable form suitable for a tax write-off.
- [ ] 4.2 Reachable by the member for their own account and by an admin for
  any member.

## 5. Email a receipt on each charge (needs an email provider)

- [ ] 5.1 On recording a payment (subscription invoice webhook or a
  Coterie-initiated charge), email the member a receipt.
- [ ] 5.2 Gate on a configured email provider: when email is unconfigured,
  skip the send silently and leave the receipt viewable in-portal. Never fail
  the payment because email is down.

## 6. Tests (offline via FakeStripeGateway)

- [ ] 6.1 Backfill idempotency: a canned set of Stripe charges imports once;
  a second run adds nothing.
- [ ] 6.2 Card de-dup: a card whose fingerprint already exists is not
  re-inserted.
- [ ] 6.3 Annual statement totals match the imported payments for a year.
- [ ] 6.4 Receipt email is attempted when email is configured and skipped
  (without error) when it is not.
