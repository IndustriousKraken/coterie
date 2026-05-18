## Why

`src/web/portal/admin/announcements.rs` (~678 lines) is the next file in the same anti-pattern that `lift-member-admin-orchestration` and `lift-event-admin-orchestration` have already fixed. Admin announcement handlers — `admin_create_announcement`, `admin_update_announcement`, `admin_delete_announcement`, `admin_publish_announcement`, `admin_unpublish_announcement` — perform their repo update + audit emission + `IntegrationEvent::AnnouncementPublished` dispatch inline.

After `lift-event-admin-orchestration` lands, the only remaining admin domain with this pattern is settings / types — and those handlers are thinner and lower-leverage. After this change, all the high-leverage admin domains (members, events, announcements) follow the service-locus rule the way `PaymentService` already did.

## What Changes

- **Add `AnnouncementAdminService`** at `src/service/announcement_admin_service.rs`. Methods:
  - `create(actor_id, input: CreateAnnouncementInput) -> Result<Announcement>` — validate + repo create + audit `create_announcement`. If `publish_now` is set, also dispatch `AnnouncementPublished` (publish path semantics).
  - `update(actor_id, announcement_id, input: UpdateAnnouncementInput) -> Result<Announcement>` — repo update + audit `update_announcement`.
  - `delete(actor_id, announcement_id) -> Result<()>` — repo delete + audit `delete_announcement`.
  - `publish(actor_id, announcement_id) -> Result<Announcement>` — flip to Published + audit `publish_announcement` + dispatch `AnnouncementPublished`.
  - `unpublish(actor_id, announcement_id) -> Result<Announcement>` — flip to Draft + audit `unpublish_announcement` (no integration dispatch — unpublish is silent today).
- **Service deps**: `Arc<dyn AnnouncementRepository>`, `Arc<AuditService>`, `Arc<IntegrationManager>`.
- **Plumb through `ServiceContext` + `AppState` + `FromRef<AppState>` impl** (same as `EventAdminService` plumbing).
- **Move orchestration out of handlers** in `src/web/portal/admin/announcements.rs`. Each handler shrinks to parse → call service → render partial.
- **Spec deltas**: `admin-announcements`, `audit-logging`, `integration-events` — same shape as the event-admin lift.

## Capabilities

### New Capabilities
- `announcement-admin-service`: single entrypoint for admin-driven announcement mutations.

### Modified Capabilities
- `admin-announcements`: handlers SHALL call `AnnouncementAdminService`.
- `audit-logging`: announcement operations join the service-locus column.
- `integration-events`: announcement dispatch moves into `AnnouncementAdminService`.

## Impact

- **Code**: new ~250-line `src/service/announcement_admin_service.rs`. `announcements.rs` shrinks by ~200 lines.
- **Wire shape**: zero change.
- **Tests**: existing tests pass unchanged. Add unit tests covering the service's side-effect chain.
- **Risk**: low. Pattern is well-established.
- **Dependency**: depends on `a05-add-fromref-impls-on-appstate` (for the FromRef impl). Position `a09` ensures this.
