## Why

Members who RSVP to events receive no automated reminder before the event. The TODO has flagged "Welcome emails for new members, event reminders, announcement digests (current emails are payment-related only)" — this change ships the event-reminder slice.

The natural shape mirrors the existing dues-reminder pattern: a flag column on the relevant row that says "we already sent a reminder for this cycle/event," a background runner method that finds eligible rows and sends, and an idempotency guard so the runner can fire frequently without spamming.

## What Changes

- **Schema**: add `reminder_sent_at: Option<DateTime<Utc>>` to the attendees table (the row that links member ↔ event for RSVPs). One reminder per attendee per event.
- **`EmailSender` template**: add `EventReminderHtml` and `EventReminderText` templates carrying event title, start time, location (if present), and a portal link.
- **`Notifications` sub-service on `BillingService`** is the existing home for transactional emails (it owns dues-reminders + the subscription-cancelled notice). Add `Notifications::send_event_reminders(lead_hours: u32) -> Result<u32>` that:
  1. Finds RSVPs whose event starts within `[now, now + lead_hours]` AND has `reminder_sent_at IS NULL`.
  2. For each, sends the email and stamps `reminder_sent_at = CURRENT_TIMESTAMP` atomically.
  3. Returns the count of reminders sent.
- **`BillingRunner` tick** adds a call to `notifications.send_event_reminders(24)` once per cycle (the runner already fires hourly).
- **Configuration**: the lead-time window is a setting `events.reminder_lead_hours` (default 24). Stored in the existing settings table; admin-editable via the existing settings page (no UI change required — the setting just needs a row in the migration).
- **Out of scope**: cancellation emails on event cancel (separate concern), recurring-series reminders (the per-occurrence row handles this naturally — each occurrence reminder gets its own `reminder_sent_at` because attendees row is per-event-id, not per-series).
- **New capability spec**: `event-reminders`.

## Capabilities

### New Capabilities
- `event-reminders`: automated email reminders to members who have RSVP'd to upcoming events. T-N-hours before the event start.

### Modified Capabilities

(None — the existing `recurring-billing` spec covers the runner pattern; no requirement of that spec changes.)

## Impact

- **Code**:
  - **New migration**: `migrations/022_event_reminder_sent_at.sql` — `ALTER TABLE event_attendees ADD COLUMN reminder_sent_at TIMESTAMP NULL;` (verify the table name; might be `event_rsvps` or `attendances`).
  - **New email template**: `src/email/templates.rs` gains `EventReminderHtml` / `EventReminderText`.
  - **Service method**: `Notifications::send_event_reminders(lead_hours)` in `src/service/billing_service/notifications.rs`.
  - **Repository method**: `EventRepository::list_pending_reminders(now, until) -> Result<Vec<(Event, Member)>>` (or similar — returns the joined attendee+event+member rows that need reminders).
  - **Repository method**: `EventRepository::mark_reminder_sent(event_id, member_id) -> Result<()>` — atomic stamp.
  - **Settings row**: add `events.reminder_lead_hours = 24` default in a migration.
  - **Runner**: `BillingRunner::run_cycle` calls `notifications.send_event_reminders(lead_hours)` after the existing dues-reminder step.
- **Wire shape**: no HTTP change. The only externally-observable change is members receiving an email roughly 24h before an event they RSVP'd to.
- **Tests**: unit tests for the runner method covering: skip-already-sent, send-and-stamp, lead-window boundary (events outside the window don't get reminders, events inside do), failure-to-send doesn't stamp.
- **Risk**: low. Mirrors the existing dues-reminder shape.
