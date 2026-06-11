## MODIFIED Requirements

### Requirement: Locus of audit emission varies by domain

Audit-log emission SHALL live EITHER in the service layer OR in the handler, depending on the domain:

- **Payments**: emitted from `PaymentService` (for manual payment recording / waiving) and `PaymentAdminService` (for admin refunds). All payment-mutation paths route through a payment-flavored service.
- **Member operations**: emitted from `MemberService`.
- **Event operations**: emitted from `EventAdminService`.
- **Announcement operations**: emitted from `AnnouncementAdminService`.
- **Settings, types**: emitted from the handler. The service-locus pattern has not yet been extended to these domains.
- **Logout**: emitted from the handler in `src/api/handlers/auth.rs`.

After this change, every admin-mutation domain except settings/types follows the service-locus rule.

#### Scenario: Refund routes through PaymentAdminService

- **WHEN** an admin refunds a payment via `/portal/admin/payments/:id/refund`
- **THEN** the `refund_payment` audit row SHALL be emitted by `PaymentAdminService::refund`, not by the handler
