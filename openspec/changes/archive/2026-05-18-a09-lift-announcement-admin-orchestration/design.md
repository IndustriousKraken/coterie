## Context

This is the third in a series of admin-orchestration lifts: `lift-member-admin-orchestration` → `lift-event-admin-orchestration` → this. The pattern is well-established at this point; this change applies it to the announcement domain.

Announcement admin handlers today do the audit + integration dispatch inline. The integration event of interest is `AnnouncementPublished`, which fires under two conditions:

1. Creation with `publish_now=true` (the create form's checkbox).
2. The explicit publish action from a previously-Draft state.

Both paths share the same Discord-channel-dispatch downstream. The service centralizes both.

## Goals / Non-Goals

**Goals:**
- One `AnnouncementAdminService` for every admin mutation.
- Handlers shrink to parse + call + render.
- Wire shape unchanged.

**Non-Goals:**
- Adding new mutations (e.g., scheduled publish — that's `a11-scheduled-announcement-publish`).
- Changing the `AnnouncementPublished` payload or downstream handling.
- Touching member-facing announcement handlers (`src/web/portal/announcements.rs` — read-only list views).

## Decisions

### D1. Service location and shape

`src/service/announcement_admin_service.rs`, `AnnouncementAdminService`. Same naming convention as `EventAdminService`.

### D2. `create` accepts `publish_now: bool` on the input

The handler today reads a checkbox to decide whether to mark the new row Published and dispatch the integration event. The service receives this as a typed field on `CreateAnnouncementInput`. If true, the service marks Published, audits `create_announcement`, and dispatches `AnnouncementPublished`. If false, the service marks Draft and only audits.

### D3. `publish` and `unpublish` are separate methods

The dedicated publish/unpublish actions transition state and (for publish only) dispatch the integration event. Keeping them as discrete methods matches the existing handler structure and the spec language.

### D4. Plumbing matches the EventAdminService precedent

Construct in `ServiceContext::new`; expose via `AppState`; add `FromRef<AppState> for Arc<AnnouncementAdminService>` impl.

### D5. Failure semantics inherit from MemberService/EventAdminService

Audit + integration failures are logged and swallowed. Repo failure propagates.

## Risks / Trade-offs

- **Risk**: the `publish_now` decision drifts during the move. → **Mitigation**: line-by-line port of the existing handler; the audit emit + integration dispatch lines are explicit pre/post-move references.
- **Trade-off**: another per-domain service. Acceptable; the per-domain pattern is now the documented convention.

## Migration Plan

Single PR.

1. Add `src/service/announcement_admin_service.rs`.
2. Plumb into `ServiceContext` + `AppState` + add `FromRef<AppState>` impl.
3. Migrate handlers one at a time: create → update → delete → publish → unpublish.
4. After each, run `cargo test --features test-utils`.
5. Confirm `announcements.rs` line count drops to ~500.
