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

### Requirement: Admin members page links to the CSV export

The admin members page (`/portal/admin/members`) SHALL include a visible "Download CSV" link that points at `/portal/admin/members/export`. The link SHALL preserve the current filter query string (e.g., if the page is filtered to `?status=Active`, the link points at `/portal/admin/members/export?status=Active`).

#### Scenario: Filter state is preserved in the export link

- **WHEN** an admin visits `/portal/admin/members?status=Expired&type=annual`
- **THEN** the page renders a "Download CSV" link with `href="/portal/admin/members/export?status=Expired&type=annual"`

#### Scenario: Link is admin-only (lives on an admin-only page)

- **WHEN** a non-admin somehow reaches the link
- **THEN** the export endpoint itself rejects the request via `require_admin_redirect`

### Requirement: Admin members page links to the bulk import flow

The admin members page (`/portal/admin/members`) SHALL include a visible "Bulk import" button or link that navigates to `/portal/admin/members/import`. The import page renders a form with a file input and a brief format reminder listing the required and optional columns.

#### Scenario: Bulk-import entry point is reachable from the members page

- **WHEN** an admin visits `/portal/admin/members`
- **THEN** the page SHALL render a "Bulk import" affordance alongside the existing "New Member" affordance

#### Scenario: Format reminder lists required and optional columns

- **WHEN** an admin visits `/portal/admin/members/import`
- **THEN** the page SHALL display the required columns (`email`, `username`, `full_name`, `membership_type_slug`) and the optional ones (`status`, `notes`, `discord_id`) clearly enough that a first-time user knows what to put in their CSV

### Requirement: Bulk-CSV admin handlers live in a sibling sub-module

The bulk-CSV admin operations (`admin_members_export`, `admin_members_import_page`, `admin_members_import`, plus their supporting templates and parse/render helpers) SHALL live in `src/web/portal/admin/members/bulk.rs`, a sub-module of the `members` admin module. The per-member admin handlers (single-member CRUD, status transitions, dues operations) SHALL live in `src/web/portal/admin/members/mod.rs`.

`members/mod.rs` SHALL re-export the public surface from `bulk` (typically via `pub use bulk::*;`) so route registration continues to resolve handler names at `admin::members::<name>` without needing to know the internal `bulk` sub-path.

The intent: `members/mod.rs` is the per-member admin page; `bulk.rs` is the roster-level bulk operations. Each file has a coherent identity. The shared parent module groups them under one URL family.

#### Scenario: New bulk-CSV handler lands in bulk.rs

- **WHEN** a contributor adds a new bulk-CSV admin operation (e.g., bulk export of payment history)
- **THEN** the handler, its template, and its helpers SHALL be added to `bulk.rs`, not to `mod.rs`

#### Scenario: New per-member handler lands in mod.rs

- **WHEN** a contributor adds a new per-member admin action (e.g., a "force-verify email" button)
- **THEN** the handler SHALL be added to `mod.rs`, not to `bulk.rs`

#### Scenario: Route registration stays flat

- **WHEN** the router file (`src/web/portal/mod.rs`) registers a bulk-CSV route
- **THEN** the handler path SHALL read `admin::members::admin_members_export` (or equivalent), NOT `admin::members::bulk::admin_members_export`; the `pub use bulk::*;` re-export flattens the path

