-- Add optional future-publish timestamp to announcements. A Draft row
-- (published_at IS NULL) with scheduled_publish_at set is "scheduled";
-- the background runner flips it to Published when the time arrives.
--
-- NULL means "not scheduled" — behavior matches today. The column is
-- ignored once the row transitions to Published (the runner clears it
-- on transition, per the spec).

ALTER TABLE announcements ADD COLUMN scheduled_publish_at DATETIME;
