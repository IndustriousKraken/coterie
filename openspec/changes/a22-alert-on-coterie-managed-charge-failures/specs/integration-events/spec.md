## MODIFIED Requirements

### Requirement: Locus of integration-event dispatch varies by domain

`IntegrationManager::handle_event` SHALL be called from EITHER the service layer OR the handler, depending on the domain:

- **Member operations**: dispatched from `MemberService`.
- **Event operations**: dispatched from `EventAdminService`.
- **Announcement operations**: dispatched from `AnnouncementAdminService`.
- **Payment / billing operations**:
  - Stripe-managed charge failures and subscription deletions: dispatched from `BillingService::Notifications` via `notify_subscription_payment_failed` and `notify_subscription_cancelled`.
  - Coterie-managed terminal charge failures: dispatched from `BillingService::AutoRenew::process_scheduled_payment`. (Per-retry transients are silent on the integration channel.)
  - Refunds: dispatched from `PaymentAdminService::refund`.
- **System notifications**: any subsystem MAY dispatch `IntegrationEvent::AdminAlert` directly.

#### Scenario: Coterie-managed terminal failure dispatches via AutoRenew

- **WHEN** a Coterie-managed scheduled payment hits the max-retries cap and transitions to `Failed`
- **THEN** the integration dispatch SHALL come from `AutoRenew::process_scheduled_payment`, not from a handler or another service
