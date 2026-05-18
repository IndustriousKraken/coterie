## 1. Schema

- [ ] 1.1 Create `migrations/024_announcement_scheduled_publish.sql` adding `scheduled_publish_at TIMESTAMP NULL` to `announcements`.
- [ ] 1.2 Update the `Announcement` struct in `src/domain/announcement.rs` to include `pub scheduled_publish_at: Option<DateTime<Utc>>`.
- [ ] 1.3 Update `SqliteAnnouncementRepository`'s row-to-domain mapping to include the new column. Update every `SELECT … FROM announcements` query to add the column. Update `INSERT` and `UPDATE` queries to handle it.

## 2. Repository methods

- [ ] 2.1 Add `AnnouncementRepository::list_due_for_publish(now: DateTime<Utc>) -> Result<Vec<Announcement>>` to the trait. Implementation: `SELECT … FROM announcements WHERE status = 'Draft' AND scheduled_publish_at IS NOT NULL AND scheduled_publish_at <= ?`.
- [ ] 2.2 Add `AnnouncementRepository::mark_published_now(id: Uuid) -> Result<bool>` — atomic conditional UPDATE. Returns true iff a row was affected.

## 3. Input shapes

- [ ] 3.1 Update `CreateAnnouncementInput` (added by `a09-lift-announcement-admin-orchestration`) to include `scheduled_publish_at: Option<DateTime<Utc>>`.
- [ ] 3.2 Update `UpdateAnnouncementInput` similarly.

## 4. Service methods

- [ ] 4.1 Update `AnnouncementAdminService::create` to persist `scheduled_publish_at` from the input. Decision: if `publish_now` is true on the input, force `scheduled_publish_at = None` (publish-now wins per spec).
- [ ] 4.2 Update `AnnouncementAdminService::update` to persist `scheduled_publish_at` from the input.
- [ ] 4.3 Add `AnnouncementAdminService::publish_scheduled() -> Result<u32>`. Body: `list_due_for_publish` → per-row `mark_published_now` → on `true` return: audit `auto_publish_announcement` with `actor_id = None`, dispatch `AnnouncementPublished`. Return success count.

## 5. Form templates

- [ ] 5.1 In `templates/admin/announcement_form.html` (or wherever the new/edit form lives), add `<input type="datetime-local" name="scheduled_publish_at" value="{{ scheduled_publish_at_display }}" />` with a small "Times are UTC" hint.
- [ ] 5.2 In `templates/admin/announcement_detail.html`, render the scheduled time if set: "Scheduled to publish at: {time}" near the status indicator.

## 6. Handlers

- [ ] 6.1 In `src/web/portal/admin/announcements.rs`, update the form-parsing code in `admin_create_announcement` and `admin_update_announcement` to read `scheduled_publish_at` from the multipart form. Empty value → `None`. Non-empty → parse `NaiveDateTime` and treat as UTC.

## 7. Runner

- [ ] 7.1 In `src/jobs/billing_runner.rs::run_cycle`, after the existing announcement/notifications steps, add `announcement_admin_service.publish_scheduled().await` with log-and-continue error handling.
- [ ] 7.2 `Notifications` or `AnnouncementAdminService` — figure out which Arc the runner already holds and which is cleanest. Likely `Arc<AnnouncementAdminService>` directly on `AppState` (since the runner is constructed from `AppState`). Verify by reading `src/jobs/billing_runner.rs` and `main.rs` together.

## 8. Tests

- [ ] 8.1 Integration test: seed a Draft with `scheduled_publish_at = now - 5 minutes`, call `publish_scheduled()`, assert: row is now Published, an `auto_publish_announcement` audit row exists with `actor_id = NULL`, the integration manager received `AnnouncementPublished` (via test fake).
- [ ] 8.2 Integration test: seed a Draft with `scheduled_publish_at = now + 2 hours`, call `publish_scheduled()`, assert: row stays Draft, no audit row, no integration event.
- [ ] 8.3 Integration test: seed a Published row with `scheduled_publish_at = now - 5 minutes` (edge case — shouldn't happen in practice but defensive), call `publish_scheduled()`, assert: row stays Published, no double-dispatch.
- [ ] 8.4 Integration test: idempotency under concurrent calls — invoke `mark_published_now(id)` twice in quick succession on the same Draft row; assert exactly one returns true.

## 9. Validate

- [ ] 9.1 `cargo build --all-targets --features test-utils` — clean.
- [ ] 9.2 `cargo test --features test-utils` — full suite passes, including the new tests.
