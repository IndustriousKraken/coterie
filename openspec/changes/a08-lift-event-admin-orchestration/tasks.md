## 1. Service skeleton

- [ ] 1.1 Create `src/service/event_admin_service.rs` with `EventAdminService` struct holding `Arc<dyn EventRepository>`, `Arc<RecurringEventService>`, `Arc<AuditService>`, `Arc<IntegrationManager>`. Add `new(...)` constructor.
- [ ] 1.2 Define the `CreateEventInput` and `UpdateEventInput` typed input structs in the same file. `CreateEventInput` includes `Option<RecurrenceRule>` for the series-vs-single decision; `UpdateEventInput` carries the editable subset of `Event` fields.
- [ ] 1.3 Register `pub mod event_admin_service;` in `src/service/mod.rs`.
- [ ] 1.4 Add `pub event_admin_service: Arc<EventAdminService>` to `ServiceContext` and construct it inside `ServiceContext::new`. Wire all four deps.
- [ ] 1.5 Add `impl FromRef<AppState> for Arc<EventAdminService>` in `src/api/state.rs` alongside the existing FromRef impls (from the earlier `add-fromref-impls-on-appstate` change).
- [ ] 1.6 Verify `cargo build` passes with the empty-shell service plumbed but unused.

## 2. Migrate `create`

- [ ] 2.1 Add `EventAdminService::create(actor_id: Uuid, input: CreateEventInput) -> Result<Event>`. Body: if `input.recurrence.is_some()`, call `recurring_event_service.materialize_series(...)` and audit `create_event_series`; else build the `Event` and call `event_repo.create(...)` and audit `create_event`. Then if visibility != AdminOnly, dispatch `IntegrationEvent::EventPublished(event)`.
- [ ] 2.2 Rewrite `admin_create_event` in `src/web/portal/admin/events.rs`: keep the multipart parsing; replace the inline repo+audit+integration chain with a single `event_admin_service.create(current_user.id, input).await` call.
- [ ] 2.3 The handler's signature drops its `State<Arc<AuditService>>` and `State<Arc<IntegrationManager>>` extractors (those go through the service now) and adds `State<Arc<EventAdminService>>`. It KEEPS `State<Arc<RecurringEventService>>` only if the handler does its own series materialization (it shouldn't after this change — the service owns that decision).
- [ ] 2.4 Add unit test `event_admin_service::tests::create_single_event_emits_full_chain` asserting repo touched, audit row inserted, integration event dispatched (for Members visibility), no series materialization.
- [ ] 2.5 Add unit test `event_admin_service::tests::create_recurring_series_materializes_and_audits`.
- [ ] 2.6 Add unit test `event_admin_service::tests::create_admin_only_event_skips_integration_dispatch`.

## 3. Migrate `update_one` and `update_series_from`

- [ ] 3.1 Add `EventAdminService::update_one(actor_id, event_id, input: UpdateEventInput) -> Result<Event>`. Body: repo update + audit `update_event`. No integration dispatch (updates are silent per existing design).
- [ ] 3.2 Add `EventAdminService::update_series_from(actor_id, series_id, from: DateTime<Utc>, input: UpdateEventInput) -> Result<u64>`. Body: `event_repo.update_series_occurrences_from(...)` + audit `update_event_series` with the count of affected rows.
- [ ] 3.3 Rewrite `admin_update_event` to inspect the radio choice and dispatch to either `update_one` or `update_series_from`.
- [ ] 3.4 Add unit tests for both methods asserting the audit row shape.

## 4. Migrate `delete_one`, `end_series`, `delete_series`

- [ ] 4.1 Add `EventAdminService::delete_one(actor_id, event_id) -> Result<()>`. Body: repo delete + audit `delete_event`.
- [ ] 4.2 Add `EventAdminService::end_series(actor_id, series_id, after: DateTime<Utc>) -> Result<u64>`. Body: `event_repo.delete_series_occurrences_after(...)` + update the series' `until_date` + audit `end_series`.
- [ ] 4.3 Add `EventAdminService::delete_series(actor_id, series_id) -> Result<()>`. Body: cascade delete (series row + all occurrences) + audit `delete_event_series`.
- [ ] 4.4 Rewrite `admin_delete_event` to dispatch to the right method based on the radio choice.
- [ ] 4.5 Add unit tests for all three methods.

## 5. Clean up

- [ ] 5.1 Remove any now-unused inline helpers in `src/web/portal/admin/events.rs` that only existed to feed the old chain.
- [ ] 5.2 Confirm no handler in `events.rs` calls `audit_service.log`, `integration_manager.handle_event`, or `event_repo.{create,update,delete,update_series_occurrences_from,delete_series_occurrences_after}` directly.
- [ ] 5.3 Eyeball the final `events.rs` line count — expected ~500–550 lines (down from ~916).
- [ ] 5.4 Run `cargo test --features test-utils` and confirm the full suite passes — including the existing recurring-event integration tests.
