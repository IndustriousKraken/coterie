# scheduled-payments Specification Delta

## ADDED Requirements

### Requirement: Pending→Processing transition is an atomic compare-and-swap

Claiming a scheduled payment for processing SHALL be an atomic compare-and-swap: the transition from `pending` to `processing` SHALL be performed by a single guarded statement (`UPDATE scheduled_payments SET status = 'processing', … WHERE id = ? AND status = 'pending'`) whose `rows_affected()` determines whether the caller won the claim. `process_scheduled_payment` SHALL NOT use a separate read-then-unconditional-write (which permits two concurrent callers to both pass a `status == pending` check and both proceed).

A caller that loses the claim (the guarded update affects zero rows) SHALL NOT charge the card, SHALL NOT create a `payments` row, and SHALL NOT extend the member's dues; it returns without side effects. This prevents a concurrency window in which two callers each mint a distinct `payment_id` for the same scheduled payment and each extend dues — the per-`payment_id` idempotency of dues extension does not protect against distinct `payment_id`s for one charge.

#### Scenario: Only one concurrent caller claims a pending scheduled payment

- **WHEN** two callers attempt to process the same `pending` scheduled payment concurrently
- **THEN** exactly one guarded `claim_for_processing` update SHALL affect one row (the winner), the other SHALL affect zero rows (the loser), and only the winner SHALL charge the card and extend dues

#### Scenario: Lost claim performs no side effects

- **WHEN** `process_scheduled_payment` is invoked for a scheduled payment whose status is already `processing` (already claimed by another worker)
- **THEN** the call SHALL return successfully without charging the card, without creating a new `payments` row, and without changing the member's `dues_paid_until`

#### Scenario: A non-pending scheduled payment is not reprocessed

- **WHEN** `process_scheduled_payment` is invoked for a scheduled payment whose status is `completed`, `failed`, or `canceled`
- **THEN** the atomic claim SHALL affect zero rows and the call SHALL perform no charge and no dues extension
