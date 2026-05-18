## 1. Service skeleton

- [ ] 1.1 Create `src/service/announcement_admin_service.rs` with `AnnouncementAdminService` struct holding `Arc<dyn AnnouncementRepository>`, `Arc<AuditService>`, `Arc<IntegrationManager>`. Add `new(...)` constructor.
- [ ] 1.2 Define `CreateAnnouncementInput` (carries `publish_now: bool`) and `UpdateAnnouncementInput` typed input structs.
- [ ] 1.3 Register `pub mod announcement_admin_service;` in `src/service/mod.rs`.
- [ ] 1.4 Add `pub announcement_admin_service: Arc<AnnouncementAdminService>` to `ServiceContext` and construct it inside `ServiceContext::new`.
- [ ] 1.5 Add `impl FromRef<AppState> for Arc<AnnouncementAdminService>` in `src/api/state.rs`.
- [ ] 1.6 `cargo build` passes with the empty-shell service plumbed but unused.

## 2. Migrate `create`

- [ ] 2.1 Add `AnnouncementAdminService::create(actor_id, input: CreateAnnouncementInput) -> Result<Announcement>`. Body: validate + persist with `status = Published if input.publish_now else Draft` + audit `create_announcement` + if Published, dispatch `IntegrationEvent::AnnouncementPublished(announcement)`.
- [ ] 2.2 Rewrite `admin_create_announcement` in `src/web/portal/admin/announcements.rs` to build `CreateAnnouncementInput` from the form and call the service.
- [ ] 2.3 Drop the handler's `State<Arc<AuditService>>` and `State<Arc<IntegrationManager>>` extractors; add `State<Arc<AnnouncementAdminService>>`.
- [ ] 2.4 Add unit tests covering: Draft path (no integration dispatch), Published-via-publish-now path (integration dispatched).

## 3. Migrate `update` and `delete`

- [ ] 3.1 Add `AnnouncementAdminService::update(actor_id, announcement_id, input: UpdateAnnouncementInput) -> Result<Announcement>`. Body: repo update + audit `update_announcement`. No integration dispatch.
- [ ] 3.2 Add `AnnouncementAdminService::delete(actor_id, announcement_id) -> Result<()>`. Body: repo delete + audit `delete_announcement`.
- [ ] 3.3 Rewrite `admin_update_announcement` and `admin_delete_announcement` to delegate.
- [ ] 3.4 Add unit tests for both.

## 4. Migrate `publish` and `unpublish`

- [ ] 4.1 Add `AnnouncementAdminService::publish(actor_id, announcement_id) -> Result<Announcement>`. Body: flip status Draft→Published (idempotent if already Published) + audit `publish_announcement` + dispatch `IntegrationEvent::AnnouncementPublished(announcement)`.
- [ ] 4.2 Add `AnnouncementAdminService::unpublish(actor_id, announcement_id) -> Result<Announcement>`. Body: flip Published→Draft + audit `unpublish_announcement`. No integration dispatch.
- [ ] 4.3 Rewrite `admin_publish_announcement` and `admin_unpublish_announcement` to delegate.
- [ ] 4.4 Add unit tests for both — including the "publish-then-publish-again does not double-dispatch" idempotency check.

## 5. Clean up

- [ ] 5.1 Confirm no handler in `announcements.rs` calls `audit_service.log`, `integration_manager.handle_event`, or `announcement_repo` directly for a mutation.
- [ ] 5.2 Remove any now-unused inline helpers in `announcements.rs`.
- [ ] 5.3 Eyeball: `announcements.rs` should drop to ~500 lines (down from ~678).
- [ ] 5.4 Run `cargo test --features test-utils` and confirm the full suite passes.
