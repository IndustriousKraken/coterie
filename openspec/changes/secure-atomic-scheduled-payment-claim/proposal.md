## Why

`BillingService::process_scheduled_payment` claims a Pending scheduled payment with a check-then-act sequence that is NOT atomic:

```rust
// src/service/billing_service/auto_renew.rs:519-540
let sp = self.scheduled_payment_repo.find_by_id(id).await? ...;
if sp.status != ScheduledPaymentStatus::Pending {       // CHECK
    return Err(...);
}
...
self.scheduled_payment_repo
    .update_status(id, ScheduledPaymentStatus::Processing, None)   // ACT
    .await?;
```

`update_status` issues `UPDATE scheduled_payments SET status = ?, … WHERE id = ?` with **no status guard** (`src/repository/scheduled_payment_repository.rs:216-221`), and the `status` column has no DB constraint enforcing the transition. So two concurrent invocations for the same `id` can both pass the `!= Pending` check and both flip the row to `Processing` — a classic TOCTOU/lost-update.

The Stripe charge itself is protected: the idempotency key is `sched-{sp.id}` (`auto_renew.rs:596`), so Stripe returns the same PaymentIntent and the member's bank is not double-charged. **But the local side effects are not idempotent across racers.** Each racer mints its own `payment_id = Uuid::new_v4()` (`auto_renew.rs:603`), inserts its own `payments` row (`auto_renew.rs:636`), and calls `extend_member_dues(payment.id, …)` (`auto_renew.rs:647`), which is idempotent only per `payment_id` (`src/repository/payment_repository.rs:472-543`). Two distinct `payment_id`s → **dues extended twice** (e.g. +2 years for one charge) and two `Completed` payment rows for a single Stripe charge.

Trigger / precondition: this is reachable whenever `process_scheduled_payment` runs concurrently for the same id — multiple app processes sharing the SQLite database, or any future concurrent/overlapping invocation of the runner. (The current single-process `BillingRunner` loop is sequential, so a default single-process deployment does not hit it today; this is a latent atomicity gap, not an externally attacker-triggered exploit.) Harm: financial-record corruption (double dues credit, duplicate completed payments) that is hard to detect and unwind.

The codebase already establishes the correct pattern elsewhere — `complete_pending_payment` and `claim_payment_for_refund` use a compare-and-swap `UPDATE … WHERE … status = '<expected>'` and check `rows_affected() == 1`. This change applies the same idiom to the scheduled-payment claim.

## What Changes

Make the Pending→Processing transition an atomic compare-and-swap so only one caller can claim a given scheduled payment:

- Add a repository method (e.g. `claim_for_processing(id) -> Result<bool>`) that runs `UPDATE scheduled_payments SET status = 'processing', last_attempt_at = ?, updated_at = ? WHERE id = ? AND status = 'pending'` and returns whether exactly one row was updated (`rows_affected() == 1`).
- In `process_scheduled_payment`, replace the `find_by_id` + `!= Pending` check + unconditional `update_status(Processing)` with: fetch the row for its data, then call `claim_for_processing(id)`; if it returns `false`, bail out (the row was already claimed/not pending) without charging.
- Leave the existing `update_status` calls for the later Failed/Completed transitions as-is (those are reached only by the single winner that holds the claim).

Add an `ADDED` requirement to the `scheduled-payments` spec that the Pending→Processing transition is an atomic compare-and-swap and that a lost claim does not charge or extend dues.

## Impact

- `src/repository/scheduled_payment_repository.rs` — add `claim_for_processing` (atomic guarded UPDATE returning a bool); add it to the `ScheduledPaymentRepository` trait and the Sqlite impl.
- `src/service/billing_service/auto_renew.rs` — `process_scheduled_payment` uses the atomic claim and returns early on a lost claim (treat as a no-op `Ok(())`, since another worker owns it).
- `openspec/specs/scheduled-payments/spec.md` — new requirement: atomic claim of a Pending scheduled payment.
- Tests: a test asserting that a second `claim_for_processing` on an already-`processing` row returns `false`, and that `process_scheduled_payment` short-circuits (no new `payments` row, no dues extension) when the claim is lost.
