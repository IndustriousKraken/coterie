# audit-logging Specification

## MODIFIED Requirements

### Requirement: Locus of audit emission varies by domain

Audit-log emission SHALL live EITHER in the service layer OR in the handler, depending on the domain:

- **Payments**: emitted from `PaymentService::record_manual`, the per-event handlers inside `WebhookDispatcher`, `BillingService::process_scheduled_payment`, and — for the one-time Stripe payment-history backfill — from the backfill routine itself, which emits its own `import_payment` and `import_payments_batch` rows (mirroring the member CSV import). Adding any OTHER new payment-recording call site WITHOUT going through one of these four would skip the audit; the `payment-recording` capability spec lists the four permitted entry points.
- **Member operations** (activate, suspend, update, extend-dues, set-dues, expire-now, create, update-discord-id, resend-verification, bulk-import): emitted from `MemberService` in `src/service/member_service.rs`. The handler in `src/web/portal/admin/members/` SHALL NOT emit audit logs directly for these operations; the service handles it.
- **Settings, types, announcements, events**: emitted from the handler. (The `admin-types` capability's audit emission is a real bug today — the spec says the handler audits, but the code doesn't. A separate change adds the missing calls.)
- **Logout**: emitted from the handler in `src/api/handlers/auth.rs`.
- **Credential and second-factor lifecycle** (self-service password change, completed password reset, TOTP enrolment, TOTP disablement, recovery-code regeneration): emitted from the handler that performs the change — `src/web/portal/profile.rs` for the logged-in flows and `src/web/templates/reset.rs` for the reset completion. These join logout as the auth-surface events that are reviewable account history, and the `auth-logging` capability specifies their accompanying log events.

Auth-surface events whose volume is controlled by an **unauthenticated** caller — failed logins, rate-limit rejections, and password-policy rejections — SHALL NOT write audit rows. They are log-only, per the `auth-logging` capability. Persisting them would let an anonymous caller drive unbounded writes against the same database that serves every request, and would bury operator-reviewable history under machine-generated noise. The dividing line is therefore not "is it security-relevant" but "is its volume attacker-controlled".

This reflects current code structure: member operations and payments follow the CLAUDE.md "side-effects in services" rule; type/setting/announcement/event ops have audit in handlers.

#### Scenario: New member-mutation method must emit its own audit row from the service

- **WHEN** a contributor adds a new member-mutation method to `MemberService`
- **THEN** the method MUST explicitly call `self.audit_service.log(...)` after the repo update; no handler-side audit wrapper exists for member operations

#### Scenario: New payment recording site routes through one of the four entry points

- **WHEN** a contributor adds a new code path that records a payment
- **THEN** it SHALL be one of `PaymentService::record_manual`, `WebhookDispatcher::handle_*`, `BillingService::process_scheduled_payment`, or the self-auditing Stripe payment-history backfill — each emits its audit row(s) internally; direct `payment_repo.create` calls from any other path are forbidden

#### Scenario: The payment-history backfill emits its own audit rows

- **WHEN** the one-time Stripe payment-history backfill imports historical charges
- **THEN** it SHALL emit an `import_payment` audit row per created payment and one `import_payments_batch` aggregate row for the run, without relying on any of the other three payment-recording sites

#### Scenario: Handler does not emit duplicate audit for member operations

- **WHEN** an admin-member handler is reviewed
- **THEN** it SHALL NOT contain a direct `audit_service.log` call for member-mutation actions; the service emits the row

#### Scenario: A credential-lifecycle change writes an audit row from its handler

- **WHEN** a member changes their own password, completes a password reset, or enrols in or disables TOTP
- **THEN** the handler performing the change SHALL call `audit_service.log`, and the row SHALL NOT contain the new credential or any part of it

#### Scenario: A failed login writes no audit row

- **WHEN** an authentication attempt is denied for any reason
- **THEN** no `audit_logs` row SHALL be written; the denial is recorded only in the log stream
