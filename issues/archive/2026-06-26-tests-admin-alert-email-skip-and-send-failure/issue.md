# tests-admin-alert-email-skip-and-send-failure

## Coverage gap

`AdminAlertEmailIntegration::handle_event`
(`src/integrations/admin_alert_email.rs:60-111`) has **no tests at all** —
it is referenced only by wiring in `src/main.rs` and
`src/integrations/mod.rs`. Two of its branches are the integration's whole
reason to exist, and both are untested:

1. **Skip when no recipient is configured.** `handle_event` reads
   `org.contact_email`, and when it is missing/empty it logs and returns
   `Ok(())` without calling the sender
   (`src/integrations/admin_alert_email.rs:64-74`).
2. **Send failure is absorbed, never propagated.** When
   `self.sender.send(...)` returns `Err`, the handler logs it and still
   returns `Ok(())` (`src/integrations/admin_alert_email.rs:107-110`).
   A template render failure is absorbed the same way (lines 100-103).

There is also the happy path: with `org.contact_email` set, an
`IntegrationEvent::AdminAlert` SHALL produce exactly one send to that
address — currently unverified.

## Acceptance criteria (against existing canon)

This pins behavior already required by the **admin-alert-email**
capability, `openspec/specs/admin-alert-email/spec.md`:

- Requirement **"Outbound admin-alert channel for security/billing
  events"**, Scenario **"Email failure does not abort the originating
  service call"**:
  > - **WHEN** an admin alert fails to send (SMTP timeout)
  > - **THEN** the originating service call SHALL still complete and the email failure SHALL be logged for observability

  Acceptance: a sender that returns `Err` SHALL cause `handle_event` to
  return `Ok(())` (the error is not propagated).

- Requirement **"Recipients are configured by setting, not hardcoded"**
  ("when `org.contact_email` is empty the channel SHALL skip sending") and
  its Scenario **"Recipient setting drives the To: list"**:
  > - **WHEN** an admin updates `org.contact_email`
  > - **THEN** subsequent admin alerts SHALL go to the new recipient without redeploy

  Acceptance: with `org.contact_email` empty the sender is **not** called;
  with it set, the alert is sent to exactly that address.

No production code changes. Test-only addition; no existing test is
modified or deleted.
