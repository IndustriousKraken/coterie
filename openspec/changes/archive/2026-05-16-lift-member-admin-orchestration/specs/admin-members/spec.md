## MODIFIED Requirements

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

### Requirement: Activation invalidates the member's existing sessions

When an admin activates a member (for instance, transitioning Pending → Active or Expired → Active), `MemberService::activate` SHALL call `auth_service.invalidate_all_sessions(member_id)` so the member's next request picks up the new status. Failure of this call SHALL be logged but SHALL NOT roll back the activation. The same contract applies to `MemberService::suspend` and `MemberService::expire_now`.

#### Scenario: Activated member is force-logged-out so next request re-evaluates status

- **WHEN** an admin activates a previously-Pending member
- **THEN** any session rows the member had SHALL be deleted; their next page load SHALL go through the login flow (and thereafter pass `require_auth_redirect`)

#### Scenario: Session invalidation owned by the service

- **WHEN** the activate / suspend / expire-now handler runs
- **THEN** the handler SHALL NOT call `auth_service.invalidate_all_sessions` directly; the service performs that call as part of its method body
