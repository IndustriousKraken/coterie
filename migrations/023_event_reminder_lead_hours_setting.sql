-- Hours before an event to send the RSVP reminder email. The runner
-- reads this value at the start of each tick, so operators can
-- adjust the lead time without a restart. Default is 24 (industry-
-- typical "remind me the day before"). Configurable via the existing
-- settings UI under the 'events' category.
--
-- INSERT OR IGNORE so re-applying against a hand-edited DB stays
-- idempotent.

INSERT OR IGNORE INTO app_settings
    (key, value, value_type, category, description, is_sensitive)
VALUES
    ('events.reminder_lead_hours', '24', 'number', 'events',
     'Hours before an event to send the RSVP reminder email to registered attendees.',
     0);
