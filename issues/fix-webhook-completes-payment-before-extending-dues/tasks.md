# Tasks

## 1. Reorder the checkout success handler so dues extend before the Completed flip

- [ ] 1.1 In `src/payments/webhook_dispatcher/checkout.rs::handle_successful_payment`,
  move the `payment_type_str` resolution (currently after the flip) to
  before any call to `complete_pending_payment` — it only reads
  `session.metadata` and `payment.kind`, so it does not depend on the flip.
- [ ] 1.2 For the **donation** branch, keep the flip as the terminal step:
  call `complete_pending_payment(payment.id, &pi_for_row)`; if it returns
  `false`, return `Ok(())` (a retry/loser of the race); otherwise log the
  donation and return `Ok(())`. (Donations have no dues post-work, so
  flip-first is correct and cannot strand anything.)
- [ ] 1.3 For the **membership** branch with a resolvable slug, call
  `billing_service.auto_renew.extend_member_dues_by_slug(payment.id,
  member_id, slug).await?` **before** `complete_pending_payment`. Then
  flip: `let won_flip = self.payment_repo.complete_pending_payment(
  payment.id, &pi_for_row).await?;`. Run the best-effort
  `reschedule_after_payment` only when `won_flip` is `true` (so only the
  caller that actually flipped reschedules — avoids double cancel/queue
  churn on the sync/webhook race). Keep `reschedule_after_payment`'s
  error soft-failed (log, do not propagate), as today.
- [ ] 1.4 For the **slug-unresolvable** branch, keep current behavior:
  flip to `Completed` and dispatch the single `AdminAlert`
  ("Checkout paid but dues not extended"). This path intentionally gives
  up on automatic extension; the flip stays terminal here.
- [ ] 1.5 Confirm that a transient error returned by
  `extend_member_dues_by_slug` now propagates (`?`) *before* the flip, so
  the row is left `Pending` and the dispatcher's claim release
  (`webhook_dispatcher/mod.rs`) lets Stripe's retry re-enter.

## 2. Reorder the PaymentIntent success handler the same way

- [ ] 2.1 In `src/payments/webhook_dispatcher/payment_intent.rs::handle_payment_intent_succeeded`,
  keep the existing metadata/member/amount cross-checks (lines ~53-125)
  ahead of any mutation — they must stay before the flip.
- [ ] 2.2 For the **non-membership** (donation/other) case, keep the flip
  terminal: `complete_pending_payment`; if `false`, return `Ok(())`;
  otherwise return `Ok(())` (no dues work).
- [ ] 2.3 For the **membership** case, resolve `member_id`, the member,
  and the slug as today, then call `extend_member_dues_by_slug(payment_id,
  member_id, &slug).await?` **before** `complete_pending_payment`. Flip
  after a successful extend, and run the best-effort
  `reschedule_after_payment` only when the flip returned `true`.
- [ ] 2.4 Preserve the existing early-return no-ops (missing
  `payment_id` metadata, malformed UUID, unknown local payment, missing
  member, unresolvable membership type) — those must still return
  `Ok(())` without flipping.

## 3. Regression tests

- [ ] 3.1 Add a test to `tests/stripe_webhook_test.rs` (using the
  `dispatch_checkout_session_completed` seam) named e.g.
  `checkout_completed_leaves_payment_pending_when_dues_extend_fails`:
  insert a `Pending` membership Payment row, dispatch a
  `CheckoutSession` whose metadata sets `payment_type=membership` and
  `membership_type_slug` to a slug **not present** in the membership-type
  registry (forces `extend_member_dues_by_slug` to return
  `AppError::NotFound`). Assert the dispatch returns `Err` AND the
  Payment row's status is still `Pending` (NOT `Completed`).
- [ ] 3.2 Extend that test to prove recovery: create the membership type
  for that slug, dispatch the same event again, and assert the Payment
  row is now `Completed` AND the member's `dues_paid_until` advanced by
  exactly one billing period (extend ran exactly once).
- [ ] 3.3 Add the analogous test for
  `dispatch_payment_intent_succeeded` in `tests/stripe_webhook_test.rs`:
  a membership PaymentIntent whose member's membership type is missing at
  first dispatch leaves the row `Pending`; once resolvable, a retry
  completes it and advances dues exactly once.
- [ ] 3.4 Keep/confirm an idempotency assertion: dispatching a
  successful membership event twice (both deliveries succeeding) advances
  `dues_paid_until` exactly once, mirroring the existing
  "invoice.paid is idempotent under Stripe retry" coverage.
