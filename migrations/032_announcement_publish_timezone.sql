-- Announcement scheduled-publish timezone correctness: the scheduled
-- publish time is a local wall-clock, not UTC. Pair it with an IANA zone
-- (frozen from org.timezone at scheduling) so the runner can derive the
-- true instant and publish at the intended org-local time instead of the
-- org-offset-early instant. See openspec change announcement-publish-timezone.
--
-- Same shape as 031_event_timezone: a pure ANNOTATION. It adds a zone
-- column and backfills it from the current org zone; it shifts no stored
-- scheduled_publish_at value. `org.timezone` already exists (inserted by
-- 031_event_timezone, which runs first). Default UTC reproduces today's
-- behavior exactly for any org that never set a zone.
ALTER TABLE announcements ADD COLUMN scheduled_publish_timezone TEXT NOT NULL DEFAULT 'UTC';
UPDATE announcements SET scheduled_publish_timezone = (SELECT value FROM app_settings WHERE key = 'org.timezone');
