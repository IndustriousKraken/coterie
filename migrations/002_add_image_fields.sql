-- Add image support to events and announcements

ALTER TABLE events ADD COLUMN image_url TEXT;
ALTER TABLE announcements ADD COLUMN image_url TEXT;
