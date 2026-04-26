-- Per-payment idempotency anchor for dues extension. Without this, a
-- webhook handler that fails after extending dues but before the
-- idempotency claim is committed (e.g. reschedule_after_payment errors)
-- causes the rollback to re-open the event, and Stripe's retry would
-- run the entire handler again — including extend_member_dues, which
-- is additive ("base_date + billing_period"). The retry would push
-- dues out by another full period, silently giving the member double
-- coverage on a single charge.
--
-- With this column in place, dues extension claims the row atomically:
-- whoever sets dues_extended_at first owns the extension; subsequent
-- attempts no-op. The transactional pattern in the repository pairs
-- this claim with the members table update so half-applied state is
-- impossible.

ALTER TABLE payments ADD COLUMN dues_extended_at DATETIME;
