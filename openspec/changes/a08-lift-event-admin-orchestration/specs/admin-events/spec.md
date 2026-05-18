## ADDED Requirements

### Requirement: Event-admin handlers route through EventAdminService

Admin event mutation handlers (`admin_create_event`, `admin_update_event`, `admin_delete_event`, plus the recurring-series variants) SHALL parse the wire shape (multipart form, path params, current user) and call `EventAdminService` for the actual mutation work. Handlers SHALL NOT call `event_repo.{create,update,delete}`, `audit_service.log`, or `integration_manager.handle_event` directly for these flows.

The wire shape (URLs, multipart bodies, HTMX response fragments) is unchanged.

#### Scenario: admin_create_event routes through the service

- **WHEN** an admin submits the new-event form
- **THEN** the handler SHALL build a `CreateEventInput` from the parsed multipart fields and call `EventAdminService::create(current_user.id, input)`; the side-effect chain runs inside the service

#### Scenario: Series-vs-single decision lives in the service

- **WHEN** the new-event form includes `repeat_kind != "none"`
- **THEN** the handler SHALL include the parsed recurrence rule on the `CreateEventInput`; the service decides whether to call `RecurringEventService::materialize_series(...)` vs. a single insert based on the input's `recurrence` field
