## Why

Today an announcement is either a Draft or Published, with no path to "publish at a specific future time." Admins who want to compose an announcement now and have it go live next Tuesday at 9am have to either (a) keep the tab open and click Publish at the right moment, or (b) draft it in their notes app and remember to come back.

The TODO has "Scheduled delivery (publish now vs. schedule for later)" as an open item. The natural shape mirrors the existing dues-reminder + event-reminder pattern: a column carrying the scheduled time, a background runner that flips Drafts to Published when their time arrives, and idempotency via the natural state transition.

## What Changes

- **Schema**: add `scheduled_publish_at: Option<DateTime<Utc>>` to the `announcements` table.
- **Admin form**: extend the new-announcement and edit-announcement forms to include an optional "Publish at" datetime field. When set, the announcement is saved as Draft with `scheduled_publish_at` populated.
- **Service method**: `AnnouncementAdminService::schedule_publish(actor_id, announcement_id, scheduled_for: DateTime<Utc>) -> Result<Announcement>` — sets `scheduled_publish_at`, audit-logs `schedule_announcement_publish`. Also `AnnouncementAdminService::clear_schedule(actor_id, announcement_id) -> Result<Announcement>` for the unset case.
- **Runner**: `AnnouncementAdminService::publish_scheduled() -> Result<u32>` finds Draft announcements whose `scheduled_publish_at <= now`, flips them to Published, audit-logs `auto_publish_announcement` (actor_id = None — system action), and dispatches `IntegrationEvent::AnnouncementPublished` for each. Wired into `BillingRunner::run_cycle`.
- **Idempotency**: the state transition is the guard. Once flipped to Published, the next tick's query excludes it (it filters on `status = Draft`).
- **Out of scope**: scheduled event publishing (could be added the same way later if anyone wants it); recurring schedules; the per-announcement Discord channel override (deferred per the existing TODO note).

## Capabilities

### New Capabilities
- `scheduled-announcement-publish`: an announcement can be created or edited with a future publish time; a background runner flips it to Published when that time arrives.

### Modified Capabilities
- `admin-announcements`: handlers accept and persist the optional `scheduled_publish_at`. The Draft / Published binary is unchanged; "scheduled" is a Draft with the timestamp set.
- `recurring-billing`: the runner gains an `auto-publish-announcements` step alongside its existing steps. (Note: this is a "recurring" feature in the scheduling sense — the spec name reflects when this lands and may want to be revisited at archive time if "recurring-billing" feels too narrow.)

## Impact

- **Code**:
  - **Migration**: `024_announcement_scheduled_publish.sql` adds the column.
  - **Domain**: `Announcement` struct gains the field; `CreateAnnouncementInput` / `UpdateAnnouncementInput` (from `a09-lift-announcement-admin-orchestration`) gain the field.
  - **Repository**: `AnnouncementRepository::list_due_for_publish(now: DateTime<Utc>) -> Result<Vec<Announcement>>` — Drafts with `scheduled_publish_at <= now`.
  - **Repository**: `AnnouncementRepository::mark_published_now(id) -> Result<bool>` — atomic flip Draft→Published, returns true on the first claimant.
  - **Service**: `AnnouncementAdminService::schedule_publish`, `clear_schedule`, `publish_scheduled` methods.
  - **Runner**: a call in `BillingRunner::run_cycle`.
  - **Templates**: the new-announcement and edit-announcement forms gain an HTML `<input type="datetime-local" name="scheduled_publish_at" />`. The detail page shows the scheduled time if set.
- **Wire shape**: small — new form field. Existing requests without the field continue to work (scheduling is opt-in).
- **Tests**: integration test boots an in-memory pool, seeds a Draft with `scheduled_publish_at` in the past, runs `publish_scheduled`, asserts the row is now Published AND the integration event was dispatched.
- **Risk**: low. The flip-Draft-to-Published transition is the same as the existing manual publish action; we're just adding a non-human trigger.
- **Dependency**: depends on `a09-lift-announcement-admin-orchestration` (the service this change extends). Position `a11` ensures this.
