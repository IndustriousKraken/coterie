## Context

The codebase already has the pattern: a column carrying state ("when is this due"), a runner that consults the column, an atomic state transition that prevents double-firing. Dues reminders use it (`dues_reminder_sent_at`); event reminders use it (after `a10`); this change applies it to announcement publishing.

The key insight is that "scheduled" doesn't need to be a new status — it's just a Draft with a future timestamp. When the runner fires, the row transitions Draft → Published the same way the manual publish action does, with the same audit log + integration dispatch downstream.

## Goals / Non-Goals

**Goals:**
- Admin can compose an announcement and set a future publish time on the same form.
- The runner publishes the announcement at the scheduled time (within one tick interval — currently hourly, so within ~1 hour of the scheduled time).
- Existing manual publish/unpublish actions are unchanged.
- A scheduled announcement that's manually published or unpublished before its scheduled time behaves correctly (manual publish wins; the scheduler skips already-Published rows).

**Non-Goals:**
- Per-minute scheduling precision. The runner ticks hourly; that's the resolution.
- Recurring schedules ("publish every Monday at 9am"). Out of scope; would be a different feature.
- Scheduled unpublish or scheduled delete.
- Cron-style scheduling expressions.

## Decisions

### D1. `scheduled_publish_at` is `Option<DateTime<Utc>>` on the announcement row

`None` means "not scheduled" — the announcement is a plain Draft or Published.

`Some(t)` on a Draft means "publish at or after t." `Some(t)` on a Published row is a legacy artifact (the runner cleared the field on publish OR the admin scheduled and then manually published — the field is irrelevant once Published).

To keep the data tidy, the runner SHALL clear `scheduled_publish_at = NULL` when it flips the row to Published. The manual publish path doesn't need to clear it (it'd already be NULL in the normal flow, and if a Published row somehow has it set, no harm — the field's only consulted on Draft rows).

### D2. Atomic Draft→Published transition

`AnnouncementRepository::mark_published_now(id)` runs a conditional UPDATE:

```sql
UPDATE announcements
SET status = 'Published', scheduled_publish_at = NULL, published_at = CURRENT_TIMESTAMP
WHERE id = ? AND status = 'Draft'
```

Returns true if a row was updated (claimed by this caller), false if not (someone else flipped it first, or the status isn't Draft). The runner uses this to avoid double-dispatching the integration event.

### D3. Runner reads-then-claims-then-dispatches per row

```rust
async fn publish_scheduled(&self) -> Result<u32> {
    let candidates = self.repo.list_due_for_publish(Utc::now()).await?;
    let mut sent = 0;
    for candidate in candidates {
        if self.repo.mark_published_now(candidate.id).await? {
            self.audit.log(None, "auto_publish_announcement", "announcement",
                           &candidate.id.to_string(), None, Some(&candidate.title), None).await;
            self.integration.handle_event(IntegrationEvent::AnnouncementPublished(candidate)).await;
            sent += 1;
        }
    }
    Ok(sent)
}
```

The `actor_id = None` on the audit row signals "system-initiated" (`audit-logging` spec already supports this).

### D4. Form input via `datetime-local` HTML input

The admin form's new field is `<input type="datetime-local" name="scheduled_publish_at">`. The browser submits the value in local timezone as `YYYY-MM-DDTHH:MM`. The handler parses with `NaiveDateTime::parse_from_str(...)` and converts to UTC via `chrono::Local::from_local_datetime(...)` or, simpler, just treats the input as UTC (the input is admin-only; admins should know to enter UTC, or we add a tiny "Times are UTC" note to the form). For first delivery, treat input as UTC; revisit if any operator complains.

Empty input → `None`. The form's `Option<String>` field maps to the request's `Option<DateTime<Utc>>` after parsing.

### D5. Service method signatures

```rust
impl AnnouncementAdminService {
    pub async fn schedule_publish(
        &self,
        actor_id: Uuid,
        announcement_id: Uuid,
        scheduled_for: DateTime<Utc>,
    ) -> Result<Announcement>;

    pub async fn clear_schedule(
        &self,
        actor_id: Uuid,
        announcement_id: Uuid,
    ) -> Result<Announcement>;

    pub async fn publish_scheduled(&self) -> Result<u32>;
}
```

The first two are admin actions (have `actor_id`); the third is the runner method (no actor — system).

`CreateAnnouncementInput` and `UpdateAnnouncementInput` also gain `scheduled_publish_at: Option<DateTime<Utc>>`. The create-with-schedule and edit-with-schedule paths go through the existing `create` / `update` methods (which set the field) rather than calling `schedule_publish` separately. `schedule_publish` is for the dedicated "add a schedule to an existing Draft" admin action, if one is added; for now, the form sets the field at create/update time.

(Actually — if the form does this at create/update time, do we need `schedule_publish` / `clear_schedule` at all? Probably not for v1. Keep them out of v1 to keep the surface small. The form-based schedule-at-create/update is sufficient. Drop `schedule_publish` / `clear_schedule` from the service surface; reduce to just `publish_scheduled` runner method.)

### D6. Idempotency under restart

The runner is the canonical source. If the server restarts mid-tick, no harm done — the next tick sees the same set of due-for-publish candidates and processes them.

### D7. Wire into BillingRunner

`BillingRunner::run_cycle` calls `announcement_admin_service.publish_scheduled().await` after the existing notifications steps. Same log-and-continue error handling.

## Risks / Trade-offs

- **Risk**: timezone confusion — admin enters "9:00 AM" expecting their local time, gets UTC. → **Mitigation**: D4's "Times are UTC" note on the form. A future change can add per-org timezone handling.
- **Risk**: an admin schedules for "5 minutes from now" expecting it to fire promptly — the runner ticks hourly. → **Mitigation**: documented in the spec; this is a scheduling tool, not a publish-now button. The form's existing "Publish now" checkbox is for that case.
- **Trade-off**: precision is `[scheduled_time, scheduled_time + tick_interval]`. Acceptable for an announcement use case.

## Migration Plan

Single PR.

1. Migration `024_announcement_scheduled_publish.sql` adds the column.
2. Update `Announcement` struct, `CreateAnnouncementInput`, `UpdateAnnouncementInput`, and the repo's `Announcement` row-mapping to include the field.
3. Add `list_due_for_publish` and `mark_published_now` to `AnnouncementRepository`.
4. Add `publish_scheduled` to `AnnouncementAdminService`.
5. Update the new- and edit-announcement form templates to include the `datetime-local` input. Update the handlers to parse and persist.
6. Wire `BillingRunner::run_cycle` to call the new method.
7. Tests covering: scheduled in past → fires; scheduled in future → doesn't fire; already Published → doesn't fire (no double-dispatch); manual publish before scheduled time wins; clearing the field by submitting an empty input.
