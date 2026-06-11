## Context

This change mirrors the `lift-member-admin-orchestration` precedent. The MemberService refactor demonstrated the pattern: handler shrinks to parse-and-render, service owns the validate → persist → audit → integration-dispatch chain.

Event admin handlers in `src/web/portal/admin/events.rs` are the next-heaviest violators of the rule. The file is ~916 lines, with the bulk of each handler being inline audit/integration-dispatch boilerplate. The mutation surface:

- `admin_create_event` — single event OR recurring-series creation (the latter delegates to `RecurringEventService` for materialization but still does its own audit + integration dispatch).
- `admin_update_event` — supports "edit this occurrence" vs. "edit this and all future occurrences" radio choice.
- `admin_delete_event` — supports "cancel just this", "end series here", "delete entire series".

The current handlers already use granular `State<Arc<…>>` extraction (presumably from earlier work), so this change doesn't have to fight the FromRef refactor as well — it inherits the granular shape and just collapses what each handler does inside.

## Goals / Non-Goals

**Goals:**
- Single canonical entrypoint (`EventAdminService`) for every admin-driven event mutation.
- Handlers parse input and render output; the service owns side-effects.
- Wire shape (URLs, form bodies, HTMX fragments, audit rows, integration events) is byte-identical.
- Pattern parity with `MemberService` so reading either tells the same story.

**Non-Goals:**
- Refactoring `RecurringEventService`. It owns series materialization and rule generation; that's a different concern from admin-handler orchestration.
- Changing audit-row content, integration-event variants, or any wire-visible behavior.
- Touching member-facing event handlers (`src/web/portal/events.rs` — RSVPs, list pages). Those don't have the admin orchestration pattern.
- Async/queued side-effects. Stays synchronous, same as `MemberService`.

## Decisions

### D1. Service module location and shape

`src/service/event_admin_service.rs`, named `EventAdminService`. The `Admin` suffix distinguishes it from any future non-admin event service surface (none today). `MemberService` doesn't carry the suffix because the member-admin module owns the only member-mutation surface; here, `RecurringEventService` already exists and owns a non-admin-flavored concern, so the suffix on `EventAdminService` keeps the two services disambiguated.

### D2. Method per admin action; granular input types

Each public method takes `actor_id: Uuid` first (mirroring `MemberService` and the audit-log-provenance contract). Inputs use small struct types (`CreateEventInput`, `UpdateEventInput`) rather than passing the wire shapes directly — the multipart parsing stays in the handler, and the service receives the typed result.

### D3. Recurring-series creation stays inside `create`

The handler today decides series vs. single by reading the `repeat_kind` form field. The service receives a `CreateEventInput` that carries an `Option<RecurrenceRule>`; if Some, the service calls `RecurringEventService::materialize_series(...)` and audits the series creation; if None, the service does the single-row insert and audits the event creation. The handler's form-parsing-into-rule logic stays in the handler.

### D4. Audit-row shape and `EventPublished` dispatch semantics are unchanged

The service emits the same audit rows the handlers emit today (`create_event`, `update_event`, `delete_event`, etc.). The `EventPublished` integration dispatch fires under the same conditions as today (currently: on event creation when visibility != AdminOnly). The doc comment on the service method records the rule so it's not duplicated as inline branching across the codebase.

### D5. Failure semantics inherit from MemberService

- Audit-log failure → logged + swallowed (no rollback).
- Integration dispatch failure → logged inside `IntegrationManager` + swallowed.
- Repository failure → propagated as `AppError`.

### D6. Plumbing matches `MemberService`

`ServiceContext::new` constructs an `Arc<EventAdminService>`; `AppState` (or `ServiceContext`) holds it; `FromRef<AppState>` impl is added in `src/api/state.rs` next to the other service impls.

### D7. Granular `State<Arc<EventAdminService>>` extraction in handlers

After this change, each event-admin handler extracts `State<Arc<EventAdminService>>` plus whatever else it needs (typically `Settings` for image paths, multipart, `CurrentUser`). The previous extractors for `AuditService` and `IntegrationManager` go away from the event-admin handlers — those collaborators are now reached through the service.

## Risks / Trade-offs

- **Risk**: subtle behavior drift in series-update propagation (the "edit this and all future" semantics are nuanced — start_time and per-row image_url stay, other fields propagate). → **Mitigation**: line-by-line port of the existing logic; the existing integration tests in `tests/recurring_event_test.rs` are the regression net.
- **Risk**: `EventPublished` dispatch fires under the wrong condition after the move. → **Mitigation**: existing tests assert this; preserve the visibility-check branch in the service exactly as it appears in the handler today.
- **Trade-off**: service constructor grows another set of dependencies. Acceptable; same pattern as `MemberService` and `BillingService`.

## Migration Plan

Single PR; pure-internal refactor.

1. Add `src/service/event_admin_service.rs` alongside existing handlers.
2. Plumb into `ServiceContext` + `AppState` + add `FromRef<AppState> for Arc<EventAdminService>`.
3. Migrate handlers one action at a time: create → update-one → update-series → delete-one → end-series → delete-series.
4. After each handler migration, run `cargo test --features test-utils`.
5. Remove the now-unused inline orchestration helpers from `events.rs` (any private fns that only existed to feed the inline chain).
6. Confirm `src/web/portal/admin/events.rs` line count drops substantially (~400 lines expected reduction).
