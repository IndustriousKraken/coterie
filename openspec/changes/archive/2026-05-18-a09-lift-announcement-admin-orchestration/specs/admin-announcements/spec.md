## ADDED Requirements

### Requirement: Announcement-admin handlers route through AnnouncementAdminService

Admin announcement mutation handlers SHALL parse the wire shape (form, path params, current user) and call `AnnouncementAdminService` for the actual mutation work. Handlers SHALL NOT call `announcement_repo`, `audit_service.log`, or `integration_manager.handle_event` directly for these flows.

Wire shape (URLs, form bodies, HTMX response fragments) is unchanged.

#### Scenario: admin_create_announcement routes through the service

- **WHEN** an admin submits the new-announcement form
- **THEN** the handler SHALL build a `CreateAnnouncementInput` from the parsed form (including the `publish_now` flag) and call `AnnouncementAdminService::create(current_user.id, input)`

#### Scenario: admin_publish_announcement routes through the service

- **WHEN** an admin clicks Publish on a Draft announcement
- **THEN** the handler SHALL call `AnnouncementAdminService::publish(current_user.id, announcement_id)`; the integration dispatch happens inside the service
