# admin-members Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Admin can create, view, and update members

Admin members SHALL manage members via the portal at `/portal/admin/members` and `/portal/admin/members/:id`. The handlers SHALL render server-side HTML; HTMX SHALL be used for partial updates. CSRF SHALL be enforced via the top-level layer; admin gating SHALL be enforced via `require_admin_redirect`.

The supported actions are:
- `GET /portal/admin/members` — listing.
- `GET /portal/admin/members/new` and `POST /portal/admin/members/new` — create.
- `GET /portal/admin/members/:id` — detail.
- `POST /portal/admin/members/:id/update` — update.
- `POST /portal/admin/members/:id/activate` — set status to Active.
- `POST /portal/admin/members/:id/suspend` — set status to Suspended.
- `POST /portal/admin/members/:id/expire-now` — force expiry immediately.
- `POST /portal/admin/members/:id/extend-dues` and `/set-dues` — adjust dues-paid-until.
- `POST /portal/admin/members/:id/resend-verification` — resend the verification email.
- `POST /portal/admin/members/:id/discord-id` — link/unlink Discord id.

#### Scenario: Update writes audit log + integration event from the handler

- **WHEN** an admin submits an update to a member's record
- **THEN** the handler SHALL call `member_repo.update(...)`, then `audit_service.log(...)`, then `integration_manager.handle_event(IntegrationEvent::MemberUpdated { old, new })`. There is NO `MemberService` wrapper; the handler owns these calls explicitly.

#### Scenario: Non-admin cannot reach the page

- **WHEN** an authenticated non-admin requests `/portal/admin/members`
- **THEN** the request SHALL be redirected to `/portal/dashboard` by `require_admin_redirect`

### Requirement: Admin actions affecting members emit Discord role updates when configured

When Discord integration is configured and a member's status, type, or admin flag changes in a way that affects role mappings, the system SHALL emit an integration event that updates the member's Discord roles.

#### Scenario: Status transition triggers role update

- **WHEN** an admin activates an Expired member
- **THEN** an integration event SHALL be emitted that the Discord integration consumes to add/remove roles

### Requirement: Member-page payment actions live on the per-member page

Manual payment recording, viewing payment history for a member, and refunding a payment SHALL be reached via:

- `GET /portal/admin/members/:id/payments`
- `GET /portal/admin/members/:id/record-payment`
- `POST /portal/admin/members/:id/record-payment`
- `POST /portal/admin/payments/:id/refund`

These pages SHALL share the same admin gate as other admin routes.

#### Scenario: Manual recording routes through PaymentService

- **WHEN** an admin records a manual payment
- **THEN** the handler SHALL call `PaymentService::record_manual` which itself emits the audit row; the handler does NOT need to call `audit_service.log` directly for this path

#### Scenario: Refund handler explicitly emits its own audit row

- **WHEN** an admin refunds a payment
- **THEN** the refund handler SHALL emit the audit-log entry directly (the refund flow does not currently route through PaymentService for audit emission)

### Requirement: Activation invalidates the member's existing sessions

When an admin activates a member (for instance, transitioning Pending → Active or Expired → Active), the handler SHALL call `auth_service.invalidate_all_sessions(member_id)` so the member's next request picks up the new status. Failure of this call SHALL be logged but SHALL NOT roll back the activation.

#### Scenario: Activated member is force-logged-out so next request re-evaluates status

- **WHEN** an admin activates a previously-Pending member
- **THEN** any session rows the member had SHALL be deleted; their next page load SHALL go through the login flow (and thereafter pass `require_auth_redirect`)

