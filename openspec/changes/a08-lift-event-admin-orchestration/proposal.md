## Why

`src/web/portal/admin/events.rs` (~916 lines) follows the same inline-orchestration anti-pattern that `lift-member-admin-orchestration` already fixed for member handlers. Every admin event mutation — `admin_create_event`, `admin_update_event`, `admin_delete_event`, plus the recurring-series update/cancel variants — performs the repo update, audit-log emission, and `IntegrationEvent::EventPublished` dispatch inline.

The CLAUDE.md rule and the precedent set by `PaymentService::record_manual` and `MemberService` is that side-effects belong in the service so handlers can't accidentally skip them. Today, adding a new admin event action requires a contributor to remember the side-effect chain; a forgotten audit row or integration dispatch is silently regressive.

This change applies the same lift to event-admin handlers. It's a mirror of `lift-member-admin-orchestration` — there's an existing, working template; the refactor is mechanical.

## What Changes

- **Add `EventAdminService`** at `src/service/event_admin_service.rs` mirroring `MemberService`'s shape. Methods:
  - `create(actor_id, input: CreateEventInput) -> Result<Event>` — validates + repo create + audit + `EventPublished` integration dispatch (the latter only for non-AdminOnly visibility per existing semantics).
  - `update_one(actor_id, event_id, input: UpdateEventInput) -> Result<Event>` — single-occurrence update + audit + (no integration event; updates are silent today per existing design).
  - `update_series_from(actor_id, series_id, from_date, input: UpdateEventInput) -> Result<u64>` — "edit this and all future" propagation + audit.
  - `delete_one(actor_id, event_id) -> Result<()>` — single-occurrence delete + audit.
  - `end_series(actor_id, series_id, after_date) -> Result<u64>` — end-series-here + audit.
  - `delete_series(actor_id, series_id) -> Result<()>` — full cascade + audit.
- **`EventAdminService` constructs from**: `Arc<dyn EventRepository>`, `Arc<RecurringEventService>`, `Arc<AuditService>`, `Arc<IntegrationManager>`. Same dependency-injection pattern as `MemberService`.
- **Plumb `member_service`-style** into `ServiceContext` + `AppState` + `FromRef<AppState>` impl (per the `add-fromref-impls-on-appstate` change that lands earlier).
- **Move orchestration out of handlers** in `src/web/portal/admin/events.rs`: each handler shrinks to parse-form → call-service → render-partial. Granular `State<Arc<…>>` extraction stays.
- **Out of scope**: refactoring `RecurringEventService` itself (its responsibility is the series-materialization rules, which are already cleanly separated from the admin-handler orchestration).
- **Spec deltas**: `admin-events`, `audit-logging`, and `integration-events` requirements update so event operations join member ops in the service-locus column.

## Capabilities

### New Capabilities
- `event-admin-service`: single entrypoint for admin-driven event mutations. Owns the validate → persist → audit → integration-dispatch chain.

### Modified Capabilities
- `admin-events`: handlers SHALL call `EventAdminService` rather than `event_repo` + `audit_service` + `integration_manager` directly.
- `audit-logging`: event operations join the service-locus column.
- `integration-events`: event dispatch moves into `EventAdminService`.

## Impact

- **Code**: new ~400-line `src/service/event_admin_service.rs`. `src/web/portal/admin/events.rs` shrinks substantially as inline orchestration moves out — expect ~400-line reduction.
- **Wire shape**: zero change. Same URLs, multipart bodies, HTMX fragments, audit rows, integration events.
- **Tests**: existing handler-level tests pass unmodified. Add unit tests for `EventAdminService` covering the side-effect chain — this is the part previously only testable via end-to-end HTTP.
- **Risk**: low. Pattern is established; the refactor is mechanical.
- **Dependency**: depends on `a05-add-fromref-impls-on-appstate` having landed (the FromRef impl for `Arc<EventAdminService>` needs to exist). Position in queue (`a08`) ensures this.
