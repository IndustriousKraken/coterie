## 1. Service skeleton

- [x] 1.1 Create `src/service/announcement_admin_service.rs` with `AnnouncementAdminService` struct holding `Arc<dyn AnnouncementRepository>`, `Arc<AuditService>`, `Arc<IntegrationManager>`. Add `new(...)` constructor.
- [x] 1.2 Define `CreateAnnouncementInput` (carries `publish_now: bool`) and `UpdateAnnouncementInput` typed input structs.
- [x] 1.3 Register `pub mod announcement_admin_service;` in `src/service/mod.rs`.
- [x] 1.4 Add `pub announcement_admin_service: Arc<AnnouncementAdminService>` to `ServiceContext` and construct it inside `ServiceContext::new`.
- [x] 1.5 Add `impl FromRef<AppState> for Arc<AnnouncementAdminService>` in `src/api/state.rs`.
- [x] 1.6 `cargo build` passes with the empty-shell service plumbed but unused.

## 2. Migrate `create`

- [x] 2.1 Add `AnnouncementAdminService::create(actor_id, input: CreateAnnouncementInput) -> Result<Announcement>`. Body: validate + persist with `status = Published if input.publish_now else Draft` + audit `create_announcement` + if Published, dispatch `IntegrationEvent::AnnouncementPublished(announcement)`.
- [x] 2.2 Rewrite `admin_create_announcement` in `src/web/portal/admin/announcements.rs` to build `CreateAnnouncementInput` from the form and call the service.
- [x] 2.3 Drop the handler's `State<Arc<AuditService>>` and `State<Arc<IntegrationManager>>` extractors; add `State<Arc<AnnouncementAdminService>>`.
- [x] 2.4 Add unit tests covering: Draft path (no integration dispatch), Published-via-publish-now path (integration dispatched).

## 3. Migrate `update` and `delete`

- [x] 3.1 Add `AnnouncementAdminService::update(actor_id, announcement_id, input: UpdateAnnouncementInput) -> Result<Announcement>`. Body: repo update + audit `update_announcement`. No integration dispatch.
- [x] 3.2 Add `AnnouncementAdminService::delete(actor_id, announcement_id) -> Result<()>`. Body: repo delete + audit `delete_announcement`.
- [x] 3.3 Rewrite `admin_update_announcement` and `admin_delete_announcement` to delegate.
- [x] 3.4 Add unit tests for both.

## 4. Migrate `publish` and `unpublish`

- [x] 4.1 Add `AnnouncementAdminService::publish(actor_id, announcement_id) -> Result<Announcement>`. Body: flip status Draft→Published (idempotent if already Published) + audit `publish_announcement` + dispatch `IntegrationEvent::AnnouncementPublished(announcement)`.
- [x] 4.2 Add `AnnouncementAdminService::unpublish(actor_id, announcement_id) -> Result<Announcement>`. Body: flip Published→Draft + audit `unpublish_announcement`. No integration dispatch.
- [x] 4.3 Rewrite `admin_publish_announcement` and `admin_unpublish_announcement` to delegate.
- [x] 4.4 Add unit tests for both — including the "publish-then-publish-again does not double-dispatch" idempotency check.

## 5. Clean up

- [x] 5.1 Confirm no handler in `announcements.rs` calls `audit_service.log`, `integration_manager.handle_event`, or `announcement_repo` directly for a mutation.
- [x] 5.2 Remove any now-unused inline helpers in `announcements.rs`.
- [x] 5.3 Eyeball: `announcements.rs` should drop to ~500 lines (down from ~678). Landed at 609 — the mutation-handler bytes shrank substantially (~70 lines removed across the five handlers), but the page-rendering scaffolding (templates, list/detail/new pages, ~270 lines) is independent of the lift and stays.
- [x] 5.4 Run `cargo test --features test-utils` and confirm the full suite passes. The only failure (`weekly_creates_about_52_occurrences`) is a pre-existing, date-dependent test on master, unrelated to this change.
