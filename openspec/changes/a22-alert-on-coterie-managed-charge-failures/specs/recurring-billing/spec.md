## ADDED Requirements

### Requirement: Terminal Coterie-managed charge failures dispatch AdminAlert

When `AutoRenew::process_scheduled_payment` exhausts the configured max-retries on a charge failure and transitions the scheduled-payment row to `Failed`, the service SHALL dispatch `IntegrationEvent::AdminAlert` so operators receive the failure via email (`AdminAlertEmailIntegration`) and/or Discord (if configured) without needing to tail logs.

The alert SHALL include enough context for triage without further lookup: member name + email, charged amount, retry count, the last failure reason from the scheduled-payment row, and a link to the member's admin detail page.

Per-retry transient failures (where `retry_count + 1 < max_retries`) SHALL NOT dispatch an AdminAlert. Operators are not alerted until a failure is terminal — same semantic as the existing `tracing::warn!` log emission (quiet on transient, loud on terminal).

#### Scenario: Terminal failure alerts operators

- **WHEN** a scheduled-payment charge fails AND `retry_count + 1 >= max_retries`
- **THEN** the scheduled-payment row SHALL transition to `Failed`, AND an `IntegrationEvent::AdminAlert` SHALL be dispatched with subject containing "Coterie-managed renewal failed (final)" and body including the member's identity, the amount, the retry count, and the last failure reason

#### Scenario: Transient failure does not alert

- **WHEN** a scheduled-payment charge fails AND `retry_count + 1 < max_retries`
- **THEN** the scheduled-payment row SHALL transition back to `Pending` for retry, AND NO AdminAlert SHALL be dispatched

#### Scenario: Member lookup failure does not block the parent operation

- **WHEN** the post-failure member re-fetch (to populate the alert body) fails (e.g., member row was deleted out-of-band)
- **THEN** the failure SHALL be logged via `tracing`, the alert SHALL be skipped, AND the parent `process_scheduled_payment` SHALL still complete normally — the scheduled-payment row is already marked `Failed`
