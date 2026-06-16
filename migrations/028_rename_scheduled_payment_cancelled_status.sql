-- Standardize the scheduled-payment "cancelled" status spelling.
--
-- The codebase uses the double-l "cancelled" everywhere else
-- (AttendanceStatus, OccurrenceExceptionKind, all prose/templates).
-- scheduled_payments was the lone single-l "canceled" holdout
-- (see the column comment in 003_payment_methods.sql). The domain enum
-- ScheduledPaymentStatus::Cancelled now serializes to "cancelled", so
-- rewrite any rows still carrying the old single-l value. There is no
-- CHECK constraint on this column, so only the data needs rewriting.
UPDATE scheduled_payments SET status = 'cancelled' WHERE status = 'canceled';
