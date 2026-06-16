## 1. Add an atomic claim to the scheduled-payment repository

- [x] 1.1 In `src/repository/scheduled_payment_repository.rs`, add a method to the `ScheduledPaymentRepository` trait: `async fn claim_for_processing(&self, id: Uuid) -> Result<bool>;`.
- [x] 1.2 Implement it on the Sqlite repo with a guarded compare-and-swap:
  ```sql
  UPDATE scheduled_payments
  SET status = 'processing', last_attempt_at = ?, updated_at = ?
  WHERE id = ? AND status = 'pending'
  ```
  Bind the current timestamp twice and the id, then return `Ok(result.rows_affected() == 1)`. Mirror the existing compare-and-swap style used by `complete_pending_payment` / `claim_payment_for_refund` in `src/repository/payment_repository.rs`.

## 2. Use the atomic claim in process_scheduled_payment

- [x] 2.1 In `src/service/billing_service/auto_renew.rs::process_scheduled_payment`, keep the initial `find_by_id(id)` fetch (the handler still needs `member_id`, `amount_cents`, `membership_type_id`, etc.), but remove the unconditional `update_status(id, Processing, None)` flip at lines ~538-540.
- [x] 2.2 After confirming the fetched row exists, call `let claimed = self.scheduled_payment_repo.claim_for_processing(id).await?;`. If `!claimed`, return `Ok(())` (another worker owns this payment, or it is no longer pending) WITHOUT charging or extending dues. This replaces the prior `if sp.status != Pending { return Err(...) }` early-return.
- [x] 2.3 Leave the subsequent `update_status(..., Failed, ...)` and `update_status(..., Completed, ...)` calls unchanged — they run only for the single claim winner.

## 4. Tests

- [x] 4.1 Add a repository test `claim_for_processing_is_single_flight`: insert a Pending scheduled payment, assert the first `claim_for_processing(id)` returns `true` and flips status to `processing`, and a second `claim_for_processing(id)` returns `false` and leaves status `processing`.
- [x] 4.2 Add a service test `process_scheduled_payment_noops_when_claim_lost`: pre-set the scheduled payment to `processing` (simulating a racer that already claimed it), call `process_scheduled_payment(id)`, and assert it returns `Ok(())` with NO new `payments` row created and NO change to the member's `dues_paid_until` (no dues extension).
