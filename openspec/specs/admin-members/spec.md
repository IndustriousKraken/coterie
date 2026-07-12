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

Mutation handlers SHALL delegate the full side-effect chain (repo update, session invalidation where applicable, audit log, integration dispatch, transactional emails) to `MemberService`. Handlers SHALL parse the wire shape (form/JSON) and render the response (HTMX fragment, redirect, flash); handlers SHALL NOT call `member_repo.update`, `audit_service.log`, `integration_manager.handle_event`, or the email sender directly for these flows.

#### Scenario: Update routes through MemberService

- **WHEN** an admin submits an update to a member's record
- **THEN** the handler SHALL call `MemberService::update(actor_id, member_id, request)` which performs the repo update, audit-log insert, and `MemberUpdated` integration dispatch internally; the handler SHALL render the response based on the returned `Result<Member>`

#### Scenario: Activate routes through MemberService

- **WHEN** an admin POSTs to `/portal/admin/members/:id/activate`
- **THEN** the handler SHALL call `MemberService::activate(actor_id, member_id)` which performs the repo update, session invalidation, audit log, `MemberActivated` integration dispatch, and welcome email internally

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

When an admin activates a member (for instance, transitioning Pending → Active or Expired → Active), `MemberService::activate` SHALL call `auth_service.invalidate_all_sessions(member_id)` so the member's next request picks up the new status. Failure of this call SHALL be logged but SHALL NOT roll back the activation. The same contract applies to `MemberService::suspend` and `MemberService::expire_now`.

#### Scenario: Activated member is force-logged-out so next request re-evaluates status

- **WHEN** an admin activates a previously-Pending member
- **THEN** any session rows the member had SHALL be deleted; their next page load SHALL go through the login flow (and thereafter pass `require_auth_redirect`)

#### Scenario: Session invalidation owned by the service

- **WHEN** the activate / suspend / expire-now handler runs
- **THEN** the handler SHALL NOT call `auth_service.invalidate_all_sessions` directly; the service performs that call as part of its method body

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

### Requirement: Admin flag is granted and revoked via an explicit update-form control

The member update form at `/portal/admin/members/:id` SHALL carry an explicit Administrator checkbox bound to the member's `is_admin` flag, routed through `MemberService::update` like every other admin-driven member mutation. Free-text content (the notes field included) SHALL NOT affect adminness. A grant or revoke SHALL write a dedicated audit entry (`grant_admin` / `revoke_admin`) with the old and new values; an unchanged flag SHALL write none. Revoking SHALL be rejected while the target is the only administrator — a zero-admin database locks all operators out and re-arms the unauthenticated `/setup` page on restart. The flag takes effect on the member's next request (the auth middleware reads `is_admin` per request); no re-login is required.

#### Scenario: Granting admin via the update form

- **WHEN** an admin submits the member update form with the Administrator checkbox checked for a non-admin member
- **THEN** the member's `is_admin` SHALL be set, a `grant_admin` audit entry SHALL be written, and the member SHALL see the admin portal on their next request without re-logging-in

#### Scenario: Revoking the last administrator is rejected

- **WHEN** the update form is submitted with the Administrator checkbox unchecked for the only member with `is_admin` set
- **THEN** the update SHALL be rejected with an explanatory error and the flag SHALL remain set

#### Scenario: Notes text never affects adminness

- **WHEN** a member's notes are saved containing the string "ADMIN"
- **THEN** the member's `is_admin` flag SHALL be unchanged — no free-text mechanism grants privileges

### Requirement: Admin can send a member a password reset

`POST /portal/admin/members/:id/reset-password` SHALL issue the member the same single-use, time-limited reset token and email that the self-service forgot-password flow issues, routed through `MemberService` and audited as `send_password_reset`. The action SHALL NOT reveal or change the member's password and SHALL NOT alter their status or sessions.

#### Scenario: Reset email goes out from the member page

- **WHEN** an admin triggers Send Password Reset on a member
- **THEN** a password-reset token SHALL be created for that member, the reset email SHALL be sent to their address, and a `send_password_reset` audit entry SHALL be written

### Requirement: Admin can delete a member without financial history

`DELETE /portal/admin/members/:id` SHALL hard-delete the member and their dependent records (profile, sessions, saved cards, scheduled payments, tokens, attendance) ONLY when the member has no payment rows. The deletion SHALL be rejected with guidance (suspend or expire instead) when payment rows exist — payment history is a ledger and never silently disappears. An admin SHALL NOT be able to delete their own account, and the last remaining administrator SHALL NOT be deletable. Deletion SHALL be audited as `delete_member` carrying the member's identifying info, and a successful deletion SHALL navigate the admin back to the members list.

#### Scenario: Test signup is deleted cleanly

- **WHEN** an admin deletes a Pending member who has no payment rows
- **THEN** the member row and their cascade-linked records SHALL be removed, a `delete_member` audit entry SHALL be written, and the admin SHALL land on the members list

#### Scenario: Member with payments is rejected with guidance

- **WHEN** an admin attempts to delete a member who has any payment row
- **THEN** the deletion SHALL be rejected with a message directing them to suspend or expire, and nothing SHALL be removed

#### Scenario: Self-deletion and last-admin deletion are rejected

- **WHEN** an admin attempts to delete their own account, or the only remaining administrator
- **THEN** the deletion SHALL be rejected and nothing SHALL be removed

