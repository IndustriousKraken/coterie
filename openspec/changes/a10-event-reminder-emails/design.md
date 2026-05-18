## Context

The existing email surface today is payment-flavored: dues reminders, payment-failed notices, refund alerts to admins. RSVPs to events get no follow-up. The TODO has flagged event reminders as a gap.

The existing dues-reminder flow is the right shape to copy:

1. `Notifications::send_dues_reminders` is the canonical pattern — scan members with `dues_reminder_sent_at IS NULL` whose dues are about to expire, send email, stamp the column atomically inside the same DB transaction as the send call.
2. The `BillingRunner` ticks hourly and calls the service. Idempotency is "stamp the column so the next tick skips the row."

This change applies the same pattern at the per-RSVP level.

## Goals / Non-Goals

**Goals:**
- Each RSVP gets exactly one reminder email before its event start.
- The cadence is configurable per-org (`events.reminder_lead_hours` setting).
- The runner is idempotent — repeat invocations don't double-send.
- Failed sends don't stamp the column; the next tick retries.

**Non-Goals:**
- Configurable reminder content per-event (one template org-wide; orgs that want richer messaging customize the template).
- Multiple reminders per RSVP (e.g., T-7-days AND T-24-hours). One per event is the established shape; multi-tier is a future change.
- Reminders for non-RSVP'd members. Only people who said they're coming get a reminder.
- Cancellation-on-event-cancel emails (separate concern).
- Reminders for past events.

## Decisions

### D1. Storage on the attendee/RSVP row, not on a separate table

Adding `reminder_sent_at` to the attendee row keeps the join surface small. The runner's query is a single SELECT against `event_attendees ⋈ events ⋈ members`. A separate `event_reminder_log` table would be over-engineered for one-reminder-per-RSVP semantics.

### D2. Configurable lead hours, default 24

`events.reminder_lead_hours` setting. Stored as a string in the settings table (existing convention); parsed to `u32` at runner time with a fallback to 24 on parse failure.

### D3. Atomic claim before send

The runner reads the candidates, sends per-row, then stamps. Two options:

- **A**: stamp BEFORE sending, then send. If send fails, the row is stamped but no email went out — bad.
- **B**: send first, then stamp. If stamp fails after send, the email went out but the row isn't stamped — the next tick re-sends. Less bad than A.
- **C**: atomic conditional UPDATE `WHERE reminder_sent_at IS NULL` returns 1-or-0, send only on the row we claimed, then keep stamp. Best — guarantees one email per row even under concurrent ticks (none today, but defensive).

Pick **C**. The repo method `mark_reminder_sent(event_id, member_id)` is the conditional update; it returns bool. Send only if the claim succeeded.

If the post-claim send fails, the row stays stamped (we already claimed it) and no email goes out. That's a known trade-off — we accept "missed reminder" over "duplicate reminder." Could be revisited later by adding a retry queue, but per the established pattern (dues reminders also accept this trade-off), it's fine for now.

### D4. Lead window is `[now, now + lead_hours]`, not `[now + lead_hours, now + lead_hours + tick_window]`

The window is generous on purpose: any RSVP within `lead_hours` of now that hasn't been reminded yet is eligible. This handles the case where a member RSVPs within the lead window (e.g., RSVPs 12 hours before an event with a 24-hour lead). The condition is "we want to remind them, even though we're past the ideal lead time."

Events further than `lead_hours` away are skipped. Events that already started are skipped (the runner filters by `start_time > now`).

### D5. Email template

`EventReminderHtml` and `EventReminderText` carry `{ event, member, portal_base_url }`. Subject: "Reminder: {event.title} is coming up". Body lists the start time (formatted in the org's timezone if available, otherwise UTC), location, and a link to `<portal_base_url>/portal/events/<event_id>`.

### D6. Soft-fail on email send

`Notifications::send_event_reminders` matches the existing pattern: log on send failure, continue with the next row. Return the success count.

## Risks / Trade-offs

- **Risk**: a transient email-provider outage causes the runner to claim rows and fail to send. → **Mitigation**: documented trade-off in D3. Operators can manually re-stamp `reminder_sent_at = NULL` for affected rows to retry. A future change could add a retry queue.
- **Risk**: an admin disables the runner mid-tick. → Not a real risk; the runner is `tokio::spawn`'d and exits with the process. Restart re-arms.
- **Trade-off**: timezone formatting. Today the codebase formats DateTime<Utc> without converting. The email shows times in UTC unless the org has a configured timezone (which it doesn't today). Acceptable — UTC is unambiguous. A future change can wire a `org.timezone` setting and format accordingly.

## Migration Plan

Single PR.

1. Migration `022_event_reminder_sent_at.sql` — confirm the actual table name (`event_attendees` or `event_rsvps` — check `migrations/001_initial.sql`). Add the column.
2. Migration `023_event_reminder_lead_hours_setting.sql` — insert the default `events.reminder_lead_hours = "24"` row into the settings table.
3. Add `EventReminderHtml` and `EventReminderText` to `src/email/templates.rs`.
4. Add `EventRepository::list_pending_reminders(now, until)` and `EventRepository::mark_reminder_sent(event_id, member_id)`. Add the trait declarations alongside any existing reminder methods in the per-file module (after `a04-move-repo-traits-to-per-file` lands).
5. Add `Notifications::send_event_reminders(lead_hours: u32) -> Result<u32>` in `src/service/billing_service/notifications.rs`.
6. Wire `BillingRunner::run_cycle` to call it after the existing dues-reminder step. Read the lead-hours setting at the start of each tick.
7. Tests covering the four scenarios (boundary, skip-already-sent, send-and-stamp, send-failure-doesn't-double-stamp-correctly).
