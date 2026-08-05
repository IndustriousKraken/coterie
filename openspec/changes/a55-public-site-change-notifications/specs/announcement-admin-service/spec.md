# announcement-admin-service Specification Delta

## MODIFIED Requirements

### Requirement: Publish path centralizes the AnnouncementPublished dispatch

`AnnouncementAdminService::publish` and the publish-now variant of `AnnouncementAdminService::create` SHALL each dispatch `IntegrationEvent::AnnouncementPublished(announcement)` after the persist.

The unpublish path SHALL dispatch `IntegrationEvent::AnnouncementUpdated` after the persist. It SHALL NOT dispatch `AnnouncementPublished`, which would be a lie about what happened.

Unpublish was previously specified as silent on the integration channel. That was
right while the only consumer was Discord, where the useful signal is "something
new exists" and there is nothing sensible to do about a withdrawal — a Discord
post cannot be recalled. It is wrong once a consumer renders Coterie's public
content on its own surface, because for that consumer the withdrawal is the
message that matters most: an addition it misses is stale content, while a
withdrawal it misses is content that stays public after an organization decided
it should not be. Silence on that channel makes every such consumer permanently
wrong with no way to notice.

Consumers for which a withdrawal is not actionable SHALL ignore the variant. That
is a consumer's decision to make and record, which the enum's exhaustive matching
forces it to make explicitly; it is not a reason to withhold the fact at the
source.

#### Scenario: create with publish_now dispatches the integration event

- **WHEN** an admin creates an announcement with `publish_now=true` on the form
- **THEN** the service SHALL mark the row Published, write the audit row, AND dispatch `AnnouncementPublished`

#### Scenario: explicit publish dispatches the integration event

- **WHEN** an admin transitions a Draft announcement to Published via the publish action
- **THEN** the service SHALL update status, write the audit row, AND dispatch `AnnouncementPublished`

#### Scenario: unpublish dispatches an update, not a publish

- **WHEN** an admin unpublishes a Published announcement
- **THEN** the service SHALL update status, write the audit row, AND dispatch `AnnouncementUpdated` carrying the status change; it SHALL NOT dispatch `AnnouncementPublished`

#### Scenario: A consumer with nothing to do on withdrawal ignores it

- **WHEN** `AnnouncementUpdated` reaches a consumer that only announces new content
- **THEN** that consumer SHALL take no action, having handled the variant explicitly rather than by a default arm
