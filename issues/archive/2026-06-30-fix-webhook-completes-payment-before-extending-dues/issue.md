# Webhook flips payment to Completed before extending dues — a transient extend failure strands the member, and Stripe's retry can't recover it

## Summary

Both successful-payment webhook handlers flip the Payment row
`Pending → Completed` **before** extending the member's dues. If the
dues-extension step then fails transiently (DB hiccup, lock timeout, a
slow membership-type lookup), the handler returns `Err`, the dispatcher
releases the idempotency claim so Stripe retries — but the row is now
`Completed`, so on retry `complete_pending_payment` returns `false` and
the handler short-circuits at the `if !won_flip { return Ok(()) }`
guard. The dues extension **never re-runs**.

Net result: the member was charged, the Payment row shows `Completed`,
but their `dues_paid_until` was never advanced — and the retry mechanism
the dispatcher relies on is defeated for this case. No `AdminAlert`
fires (that path only triggers when the membership-type *slug* can't be
resolved, not when the extend step itself errors). The member silently
expires later despite having paid.

This is a behavior-preserving correction: the canonical stripe-webhook
contract already requires that a transient handler failure be
*recoverable on Stripe retry*, and that dues advance *exactly once*. The
current ordering violates both. No observable contract changes — hence
the issue lane (no `specs/` delta).

## Source location

`src/payments/webhook_dispatcher/checkout.rs:47-58` and `:148-153`
(`handle_successful_payment`):

```rust
let won_flip = self
    .payment_repo
    .complete_pending_payment(payment.id, &pi_for_row)   // <-- flip FIRST
    .await?;
if !won_flip {
    // ... retry short-circuits here ...
    return Ok(());
}
// ... later, the step that can fail ...
billing_service
    .auto_renew
    .extend_member_dues_by_slug(payment.id, member_id, slug)   // <-- extend SECOND
    .await?;
```

`src/payments/webhook_dispatcher/payment_intent.rs:128-139` and
`:191-194` (`handle_payment_intent_succeeded`) — identical ordering:
`complete_pending_payment` (flip) at 128, early-return on `!won_flip` at
132-139, `extend_member_dues_by_slug` at 191-194.

Supporting facts (verified):

- `PaymentRepository::complete_pending_payment`
  (`src/repository/payment_repository.rs:397-415`) runs
  `UPDATE payments SET status='Completed' ... WHERE id=? AND status='Pending'`
  and returns `true` only when one row was flipped — so it returns
  `false` forever after the first flip.
- `AutoRenew::extend_member_dues` →
  `PaymentRepository::extend_dues_for_payment_atomic(payment_id, ...)`
  (`src/service/billing_service/auto_renew.rs:843-879`) is an **atomic,
  idempotent per-`payment_id` claim** (`UPDATE ... WHERE
  dues_extended_at IS NULL`). It cannot double-extend across retries.
  Because it is idempotent, it is safe to run it *before* the flip and
  to let Stripe's retry re-run it until it succeeds.
- The dispatcher's rollback (`webhook_dispatcher/mod.rs:181-199`)
  `release`s the idempotency claim on handler error so Stripe retries —
  which is the recovery path that the already-flipped row defeats.

The fix is to make the irreversible `Completed` flip the **last**
must-succeed step in the membership path: extend dues first (idempotent,
retry-safe), then flip. A transient extend failure then leaves the row
`Pending`, so the next Stripe retry re-enters and recovers it, and the
Pending-payments admin review can also see it. The donation path (whose
only post-work *is* the flip) and the slug-unresolvable path (which
deliberately gives up and alerts an operator) keep the flip as their
terminal step.

## Why this is harmful (trigger and impact)

- **Trigger:** any `checkout.session.completed` or
  `payment_intent.succeeded` for a membership payment where the row flip
  succeeds but the subsequent dues-extension (or the membership-type
  lookup it performs) errors transiently. Transient DB errors under load
  are routine; this is not an exotic input.
- **Impact:** money taken, Payment row `Completed`, dues not extended,
  no alert, and the very Stripe-retry mechanism meant to recover the
  failure is turned into a permanent no-op. The member's access lapses
  later with no record that anything went wrong.

## Acceptance criteria (against the EXISTING specification)

These restate guarantees already in canon; the fix makes the
checkout/PaymentIntent handlers conform to them. No spec delta.

- Conforms to **stripe-webhook → "Failed processing releases the claim
  for retry" / scenario "Transient handler failure is retried on next
  delivery"**: when the dues-extension step fails for a membership
  payment, a subsequent Stripe retry of the same event SHALL extend the
  member's dues. The payment row SHALL NOT be left `Completed` with dues
  unextended; on a transient extend failure the row SHALL remain
  `Pending` so the retry can recover it.
- Conforms to **stripe-webhook → "Event processing is idempotent via
  atomic claim"** (and the invoice analog "invoice.paid is idempotent
  under Stripe retry — dues advance exactly once"): across the original
  delivery plus any retries, the member's `dues_paid_until` SHALL advance
  **exactly once** for a single payment — never twice.
- The donation path and the membership-type-slug-unresolvable path
  retain their current observable behavior (donation: flip + log, no
  dues work; unresolvable slug: flip + single `AdminAlert`).
