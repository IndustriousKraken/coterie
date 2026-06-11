## Context

`src/web/portal/admin/members.rs` is 1586 lines. ~10 admin handlers each do the same orchestration shape inline:

```
parse input  →  repo update  →  invalidate sessions  →  audit log
            →  integration dispatch  →  send email  →  render partial
```

The single helper that exists, `dispatch_member_updated`, only covers the integration-dispatch step and only some handlers use it. Audit logging, session invalidation, and email sending are open-coded in every handler.

The architecture rule in `CLAUDE.md` is explicit: *"Side-effects (audit log, integration events) belong in the service so handlers can't accidentally skip them."* `PaymentService::record_manual` follows this rule today — it owns validation, persistence, audit emission, and dues extension. The doc comment on that service captures the motivation precisely: *"It owns the validation + persist + dues-extension + audit-log chain that was duplicated across three handlers before consolidation."*

The same consolidation has not happened for member operations. The current `admin-members`, `audit-logging`, and `integration-events` specs describe the gap as observed-but-aspirational: *"the CLAUDE.md 'side-effects in services' rule is aspirational; payments follow it, the rest do not."*

The risk is real but not acute: any new admin action that forgets one piece of the chain (e.g., an audit row, a session invalidation, a Discord role re-sync) will silently regress without a test failing. The chain is uniform enough that it should be enforced by types.

## Goals / Non-Goals

**Goals:**
- Single canonical entrypoint (`MemberService`) for every admin-driven member mutation.
- Handlers shrink to: parse input, call service, render response.
- Wire shape (URLs, form bodies, HTMX fragments, audit row contents, integration events emitted) is byte-for-byte unchanged.
- Unit-testable side-effect orchestration without spinning up an HTTP layer.
- Pattern parity with `PaymentService` so a future event/announcement/settings/types lift has a clear template.

**Non-Goals:**
- Lifting orchestration for events, announcements, types, or settings handlers (separate, follow-up changes).
- Touching `admin_refund_payment` (lives in `members.rs` for routing reasons but operates on payments — better consolidated under `PaymentService` in a separate change).
- Changing the audit row schema, integration event variants, or any wire-visible behavior.
- Async/queued side-effects (the Outbox pattern noted in `ARCHITECTURE-NOTES.md`). Side-effects stay synchronous in the request path, same as today. Moving them to a service makes a future lift to an outbox easier, but that's not this change.
- Consolidating the "old/new snapshot for `MemberUpdated`" pattern beyond what handlers already do — the service inherits the existing semantics.

## Decisions

### D1. New service module, not new methods on existing services

`MemberService` is its own module at `src/service/member_service.rs`. It composes the dependencies it needs (`MemberRepository`, `AuthService` for session invalidation, `AuditService`, `IntegrationManager`, `EmailSender`, `MembershipTypeService`, `SettingsService` for org-name lookup in emails, `EmailTokenService` for verification resends).

Considered: putting these methods on `MemberRepository`. Rejected — the repo trait is for persistence; mixing in audit/integration/email blurs the layering and would force in-memory test fakes to also fake those side-effects.

Considered: putting them on a generic `AdminService`. Rejected — `PaymentService` is per-domain, and the prospective follow-ups (event-admin, announcement-admin) are also per-domain. Mirroring keeps the pattern legible.

### D2. Service methods return the post-update `Member`, not just `()`

Handlers need the new member to render the updated row. `PaymentService::record_manual` already returns `Result<Payment>`; we mirror. Some methods (e.g., `expire_now`) currently re-fetch the member via `dispatch_member_updated` after the update — the service can return the new member directly, eliminating a roundtrip.

### D3. Audit log failures stay swallowed

`AuditService::log` already returns `()` (per the `audit-logging` spec: *"the call SHALL return without error and the failure SHALL be logged at error level"*). The service inherits this contract — a failed audit insert does not roll back the underlying member mutation. Same semantics as today.

### D4. Integration dispatch stays fire-and-forget

`IntegrationManager::handle_event` already returns `()` and logs per-integration failures. The service inherits this contract. Same semantics as today.

### D5. Email failures stay logged-but-non-fatal

`send_welcome_email` and the verification-resend path today use `tracing::error!` and continue. The service inherits this — activate-then-email-fails still flips the member to Active and returns success, with the email failure logged. An admin can resend manually. Matches existing handler behavior.

### D6. Session invalidation on status change stays in-band

Currently handlers call `auth_service.invalidate_all_sessions(...)` in-line for status-changing operations (activate, suspend, expire). The service does the same. Session invalidation failures are logged but do not roll back the status change — middleware re-validates status per-request, so a stale session is denied access on its next hit even without invalidation.

### D7. The actor_id is a required parameter on every mutation

Every `MemberService` mutation method takes `actor_id: Uuid` as the first parameter. This makes audit-row provenance impossible to forget — there is no way to call the service without an actor. Mirrors `PaymentService::record_manual` which already does this via `RecordManualPaymentInput::actor_id`.

### D8. Validation stays in the service, not the handler

`extend_dues` validates `1..=120`. `update_discord_id` validates the snowflake format. Today these checks live in handlers; they move to the service. Handlers still parse the wire shape (form/JSON), but the domain-level validation is uniform regardless of caller — same precedent as `PaymentService` validating `amount_cents` and the donation campaign reference.

### D9. Welcome-email helper becomes a private method

`send_welcome_email` (admin-create + admin-activate paths) and `dispatch_member_updated` move into `MemberService` as private methods. The public surface only exposes the high-level operations.

### D10. `member_service` lives on `AppState` and is constructed in `ServiceContext`

Same plumbing as `payment_service` today. `ServiceContext::new` constructs it during startup; `AppState` holds an `Arc<MemberService>`. Handlers reach it via `state.service_context.member_service`.

### D11. The handler's HTML rendering stays in the handler

The service returns domain types and propagates `AppError`. The handler continues to map success / error to `partials::admin_alert(...)`, `partials::member_row_*(...)` etc. This boundary is intentional: rendering varies by route (member-row HTMX fragment vs. admin-alert flash vs. redirect-and-trigger), so it stays in the route handler.

### D12. `admin_create_member` returns the new member

Today it directly emits the welcome email and shows a flash. The service version: persists, sends welcome email, audit-logs. The activation event is intentionally *not* fired here — `MemberActivated` is reserved for the activate transition (matching today's behavior; the create path uses Pending-by-default and only emits an event once activation happens).

## Risks / Trade-offs

- **Risk**: A subtle behavior drift slips in during the move (e.g., audit row shape changes, integration event ordering shifts, email content differs). → **Mitigation**: line-by-line port; existing tests assert HTTP responses; add new service-level unit tests that assert on the recorded audit row, integration events fired, and emails sent (using the existing fakes).
- **Risk**: Compilation breakage cascades because `MemberService` pulls in many deps. → **Mitigation**: incremental landing — add the service alongside existing handlers first, migrate one handler at a time, delete the inline orchestration only when each migration is verified.
- **Risk**: Spec deltas land before the implementation, leaving the spec out of sync with code. → **Mitigation**: implementation tasks complete before specs are archived (the OpenSpec workflow already enforces this — specs sync at archive time via `opsx:archive`).
- **Trade-off**: Service constructors get more parameters. `MemberService::new` will take ~7 deps. Acceptable — same pattern as `BillingService` and `PaymentService` already in the tree. If this gets uncomfortable we can revisit `ServiceContext` ergonomics later.
- **Trade-off**: An extra layer of indirection for callers reading a single handler. The legibility win at orchestration-correctness time outweighs the cost of one extra "go-to-definition" hop.
- **Trade-off**: The "old member snapshot for `MemberUpdated`" pattern still requires a pre-update read inside the service. We're not eliminating that DB round-trip; we're moving it. Could collapse to a single SQL `RETURNING` later, but that's a repository change, not a service change.

## Migration Plan

This is a pure-internal refactor with no wire-shape change, so deployment is single-step:

1. Land the new `MemberService` module + `AppState` field + handler migrations + spec deltas in one PR.
2. Existing handler-level tests assert the wire shape; they pass without changes.
3. New `MemberService` unit tests cover the side-effect chain.
4. Deploy normally — no migrations, no feature flags, no rollout staging needed. The change is fully reversible by `git revert` if a regression is found post-deploy.
