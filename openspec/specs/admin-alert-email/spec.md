# admin-alert-email Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Outbound admin-alert channel for security/billing events

The system SHALL provide an outbound admin-alert email channel used for events that warrant operator attention (e.g., repeated billing failures, security configuration changes, suspicious login patterns). The channel SHALL use the configured SMTP / email-provider settings.

#### Scenario: Repeated charge failure produces an admin alert

- **WHEN** the billing runner records the configured threshold of consecutive failures for a member
- **THEN** an admin-alert email SHALL be sent summarizing the member, recent attempts, and a link to the per-member page

#### Scenario: Email failure does not abort the originating service call

- **WHEN** an admin alert fails to send (SMTP timeout)
- **THEN** the originating service call SHALL still complete and the email failure SHALL be logged for observability

### Requirement: Recipients are configured by setting, not hardcoded

The recipient SHALL come from the `org.contact_email` setting (the org's configured contact address), re-read per event. Hardcoding recipients in source SHALL be forbidden; when `org.contact_email` is empty the channel SHALL skip sending.

#### Scenario: Recipient setting drives the To: list

- **WHEN** an admin updates `org.contact_email`
- **THEN** subsequent admin alerts SHALL go to the new recipient without redeploy

