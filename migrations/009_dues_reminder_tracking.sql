-- Track whether a "dues expiring soon" reminder email has been sent for
-- the member's current dues cycle. Cleared whenever dues_paid_until is
-- extended (payment received), then set again when the next reminder
-- fires. One reminder per cycle.
--
-- No backfill — existing active members will be eligible for their
-- next natural reminder when their dues enter the reminder window.

ALTER TABLE members ADD COLUMN dues_reminder_sent_at DATETIME;
