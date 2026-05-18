## 1. Schema and settings

- [ ] 1.1 Identify the attendee table name by reading `migrations/001_initial.sql` (likely `event_attendees`, but verify). All later tasks should reference the correct table name.
- [ ] 1.2 Create `migrations/022_event_reminder_sent_at.sql` adding `reminder_sent_at TIMESTAMP NULL` to that table.
- [ ] 1.3 Create `migrations/023_event_reminder_lead_hours_setting.sql` that `INSERT INTO settings (key, value) VALUES ('events.reminder_lead_hours', '24')` (idempotent via `ON CONFLICT DO NOTHING` or similar — match the existing settings-insert pattern in earlier migrations).
- [ ] 1.4 `cargo sqlx prepare` or equivalent if the project uses compile-time-checked queries (verify via `grep` for `sqlx::query!`); if it uses runtime queries (the dominant pattern in this codebase), no prep step needed.

## 2. Email templates

- [ ] 2.1 Add `EventReminderHtml` and `EventReminderText` Askama templates to `src/email/templates.rs`. Match the shape of the existing dues-reminder templates (`DuesReminderHtml`, `DuesReminderText`). Carry `event: &Event`, `member: &Member`, `portal_base_url: &str`.
- [ ] 2.2 Create the template HTML at `templates/email/event_reminder.html` and text at `templates/email/event_reminder.txt`. Look at the existing `templates/email/dues_reminder.{html,txt}` for the styling/conventions.
- [ ] 2.3 The subject line should be `"Reminder: {} is coming up".format(event.title)`. Build it as a function or `Display` impl on the template struct — match the pattern other templates use.

## 3. Repository methods

- [ ] 3.1 Add `EventRepository::list_pending_reminders(now: DateTime<Utc>, until: DateTime<Utc>) -> Result<Vec<EventReminderRow>>` to the trait in `src/repository/event_repository.rs` (after `a04-move-repo-traits-to-per-file` lands, the trait lives there alongside the impl).
- [ ] 3.2 Define `EventReminderRow` as a small struct holding `event_id: Uuid`, `event_title: String`, `event_start: DateTime<Utc>`, `event_location: Option<String>`, `member_id: Uuid`, `member_email: String`, `member_full_name: String`. Place it next to the trait.
- [ ] 3.3 Implement `list_pending_reminders` as a JOIN across the attendees table, events, and members, with WHERE clauses filtering by `start_time > now AND start_time <= until AND reminder_sent_at IS NULL AND <attendance status is "yes">`. Confirm the exact attendance-status column name and value by reading the attendees-table schema.
- [ ] 3.4 Add `EventRepository::mark_reminder_sent(event_id: Uuid, member_id: Uuid) -> Result<bool>` to the trait — returns true if the conditional UPDATE affected exactly one row (i.e., `WHERE reminder_sent_at IS NULL`).

## 4. Notification service method

- [ ] 4.1 In `src/service/billing_service/notifications.rs`, add `pub async fn send_event_reminders(&self) -> Result<u32>`. Body:
  - Read `events.reminder_lead_hours` from `settings_service`, defaulting to 24 on missing or unparseable.
  - Compute `now = Utc::now()` and `until = now + Duration::hours(lead_hours)`.
  - Call `event_repo.list_pending_reminders(now, until)` to get candidate rows.
  - For each candidate: call `event_repo.mark_reminder_sent(event_id, member_id)`. If the claim returns true, send the email via `email_sender.send(...)`. Log on send failure but do not unstamp.
  - Return the count of successful sends.
- [ ] 4.2 `Notifications` already holds `Arc<dyn EmailSender>` and `Arc<SettingsService>`; check whether it needs `Arc<dyn EventRepository>` added (likely yes — add it to the struct and constructor).
- [ ] 4.3 Update `BillingService::new` to pass `event_repo` to `Notifications::new`.

## 5. Wire the runner

- [ ] 5.1 In `src/jobs/billing_runner.rs::run_cycle`, after the existing dues-reminder call, add `billing_service.notifications.send_event_reminders().await` with the same log-and-continue error handling shape.

## 6. Tests

- [ ] 6.1 Add an integration test `tests/event_reminder_test.rs` that boots an in-memory SQLite pool, seeds a member, an event starting in 6 hours, an attendees row, runs the service method, and asserts: one email queued (via a fake `EmailSender`) AND the attendees row's `reminder_sent_at` is now set.
- [ ] 6.2 Same test, but the event starts in 48 hours — asserts NO email queued and `reminder_sent_at` stays NULL.
- [ ] 6.3 Same test, but the row is already stamped — asserts NO email queued.
- [ ] 6.4 Same test, but the email sender's mock returns an error — asserts the row stays stamped (per the design's claim-then-send semantics) and the failure is logged.

## 7. Validate

- [ ] 7.1 `cargo build --all-targets --features test-utils` — clean.
- [ ] 7.2 `cargo test --features test-utils` — full suite passes.
