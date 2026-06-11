-- Track whether a "your event is coming up" reminder has been sent
-- for this attendance row. One reminder per RSVP per event. NULL
-- means eligible; CURRENT_TIMESTAMP means claimed-and-sent (per the
-- claim-then-send semantics in event-reminders spec).
--
-- No backfill — existing RSVPs to upcoming events become eligible on
-- the next runner tick.

ALTER TABLE event_attendance ADD COLUMN reminder_sent_at DATETIME;
