# member-admin-service Specification

## Purpose
TBD - created by archiving change lift-member-admin-orchestration. Update Purpose after archive.
## Requirements
### Requirement: MemberService is the single entrypoint for admin-driven member mutations

The system SHALL expose a `MemberService` at `src/service/member_service.rs` that owns the full side-effect chain (repo update, session invalidation, audit log, integration dispatch, transactional emails) for every admin-driven member mutation. Admin handlers SHALL call this service rather than invoking the repository, audit service, integration manager, and email sender directly. Adding a new admin member action WITHOUT extending `MemberService` SHALL be treated as a defect.

#### Scenario: Handlers call the service, not the repo + collaborators

- **WHEN** an admin POSTs to a member-mutation route (`/portal/admin/members/:id/activate`, `/suspend`, `/update`, `/extend-dues`, `/set-dues`, `/expire-now`, `/discord-id`, `/resend-verification`, `/new`)
- **THEN** the handler SHALL call exactly one `MemberService` method to perform the operation; the handler SHALL NOT call `member_repo.update`, `audit_service.log`, or `integration_manager.handle_event` directly for that flow

#### Scenario: Forgetting an audit row is structurally impossible

- **WHEN** a contributor adds a new method to `MemberService` for a new admin action
- **THEN** the method SHALL emit the audit row, integration event, and (if applicable) email and session invalidation as part of its body; handlers calling the new method inherit all side-effects without per-handler wiring

### Requirement: Every mutation method takes an explicit actor_id

`MemberService` mutation methods SHALL take `actor_id: Uuid` (the acting admin's member id) as a required parameter. The method SHALL pass `actor_id` to `audit_service.log` so audit-row provenance cannot be omitted by the caller.

#### Scenario: Audit row carries actor

- **WHEN** an admin invokes any `MemberService` mutation
- **THEN** the resulting `audit_logs` row SHALL have `actor_id = <admin's member uuid>`

### Requirement: Service methods return the post-update Member

Mutation methods SHALL return `Result<Member>` so handlers do not need to re-fetch to render the updated row. Methods that don't naturally return a member (e.g., `resend_verification`) SHALL return `Result<()>`.

#### Scenario: Activate returns the new Active member

- **WHEN** a handler calls `MemberService::activate(actor_id, member_id)`
- **THEN** the returned `Member` SHALL reflect `status = Active` and SHALL be the value the handler passes to the response renderer

### Requirement: Service inherits existing failure semantics

`MemberService` SHALL preserve the failure semantics already in place:

- Audit-log insert failure: logged via `tracing`, swallowed (no error propagated).
- Integration dispatch failure: per-integration failures logged inside `IntegrationManager`; the call returns success.
- Email send failure: logged via `tracing`, swallowed (the primary mutation already succeeded).
- Session invalidation failure: logged via `tracing`, swallowed (middleware re-validates per-request).
- Repository failure: propagated as `AppError` to the caller.

#### Scenario: Email failure does not roll back activation

- **WHEN** `MemberService::activate` succeeds at the repo update but the welcome-email send fails
- **THEN** the method SHALL return `Ok(member)` with the activated member; the email failure SHALL be logged at error level

#### Scenario: Repo failure surfaces to handler

- **WHEN** the underlying `member_repo.update` returns an error
- **THEN** `MemberService::activate` SHALL return that error; no audit row, integration event, or email SHALL be emitted

### Requirement: MemberService is plumbed through ServiceContext and AppState

`ServiceContext::new` SHALL construct `MemberService` and expose it via `Arc<MemberService>`. `AppState` SHALL hold this Arc so handlers reach it via `state.service_context.member_service`. Plumbing SHALL mirror `payment_service`'s shape.

#### Scenario: Handlers reach the service via AppState

- **WHEN** any admin member-mutation handler runs
- **THEN** it SHALL access the service via `state.service_context.member_service.<method>(...)`; it SHALL NOT construct a service instance per-request

