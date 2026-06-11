## ADDED Requirements

### Requirement: RSVP'd members receive one reminder email before an event

For every event whose start time falls within `[now, now + lead_hours]`, the system SHALL email each member who has an active RSVP a reminder, EXACTLY ONCE per RSVP. The lead-hours window is configurable via the `events.reminder_lead_hours` setting (default 24).

The reminder SHALL include the event title, formatted start time, location (if present on the event), and a link to the event's portal detail page.

#### Scenario: RSVP'd member receives a reminder

- **WHEN** a member has an active RSVP for an event starting in 23 hours AND the lead-hours setting is 24
- **THEN** the next runner tick SHALL email that member; the attendee row SHALL be stamped with `reminder_sent_at = now`

#### Scenario: Already-stamped row is skipped

- **WHEN** the runner ticks again after a reminder was sent
- **THEN** the same RSVP SHALL NOT generate a second email; the row's `reminder_sent_at` is non-null

#### Scenario: Event outside the lead window is skipped

- **WHEN** an event starts in 48 hours AND the lead-hours setting is 24
- **THEN** no reminder SHALL be sent on this tick; the next tick will re-evaluate as the window approaches

#### Scenario: Past events are not reminded

- **WHEN** an event's start time is in the past
- **THEN** no reminder SHALL be sent regardless of `reminder_sent_at` status

#### Scenario: Late RSVP inside the window still gets a reminder

- **WHEN** a member RSVPs to an event 6 hours before its start AND the lead-hours setting is 24
- **THEN** the next runner tick SHALL include this RSVP and send the reminder (the window is `[now, now + lead_hours]`, generous on the recent-RSVP side)

### Requirement: Reminder claim and send is atomic per RSVP

The runner SHALL claim each RSVP for reminding via an atomic conditional UPDATE (`SET reminder_sent_at = now WHERE reminder_sent_at IS NULL`) that returns whether the row was claimed. The email SHALL be sent only after a successful claim, so two concurrent runner ticks cannot both email the same RSVP.

If the post-claim email send fails, the row stays stamped — the RSVP will not be re-reminded. This is a documented trade-off (preferring "rare missed reminder" over "occasional duplicate reminder"). Operators can manually clear `reminder_sent_at` to retry.

#### Scenario: Claim before send prevents duplicate emails

- **WHEN** the runner identifies an eligible RSVP
- **THEN** it SHALL conditionally stamp `reminder_sent_at` first; only on a successful row-count-1 update SHALL it send the email

#### Scenario: Send failure does not unblock a re-attempt

- **WHEN** the email send fails after a successful claim
- **THEN** the row stays stamped; the failure is logged via `tracing`; operator intervention (clearing the column) is required to retry

### Requirement: Lead-hours setting is read on each tick

`Notifications::send_event_reminders` SHALL read the `events.reminder_lead_hours` setting at the start of each invocation. A missing or unparseable setting SHALL default to 24. This means operators can adjust the lead time without restarting the server.

#### Scenario: Setting change takes effect on the next tick

- **WHEN** an admin updates `events.reminder_lead_hours` from 24 to 48
- **THEN** the next runner tick SHALL use 48 as the lead window
