## Why

The CLAUDE.md architectural rule states *"Side-effects (audit log, integration events) belong in the service so handlers can't accidentally skip them."* Today, member-admin handlers in `src/web/portal/admin/members.rs` (1586 lines) violate this rule: every action — `activate`, `suspend`, `update`, `extend-dues`, `set-dues`, `expire-now`, `update-discord-id`, `resend-verification`, `create` — performs the repo update, session invalidation, audit log, integration dispatch, and email send chain inline. The current `admin-members`, `audit-logging`, and `integration-events` specs explicitly codify this as observed-but-aspirationally-wrong (*"the CLAUDE.md 'side-effects in services' rule is aspirational; payments follow it, the rest do not"*).

`PaymentService::record_manual` already demonstrates the right shape — handler parses input, service owns the validate + persist + audit + extend-dues chain. We extend that pattern to member operations so the side-effect chain is in one testable place and a future handler that forgets a step is structurally impossible.

## What Changes

- **Add `MemberService`** at `src/service/member_service.rs` owning the full side-effect chain for admin-driven member mutations. Mirrors `PaymentService`'s shape.
- **Move orchestration out of handlers** in `src/web/portal/admin/members.rs`: each admin action handler shrinks to parse-input → call-service → render-partial. The repo / audit_service / integration_manager calls move into the service.
- **Methods on `MemberService`**:
  - `activate(actor_id, member_id) -> Result<Member>` — repo update + invalidate sessions + audit + `MemberActivated` integration event + welcome email
  - `suspend(actor_id, member_id) -> Result<Member>` — old/new snapshot + repo update + invalidate sessions + audit + `MemberUpdated` event
  - `update(actor_id, member_id, UpdateMemberRequest) -> Result<Member>` — old/new snapshot + repo update + audit + `MemberUpdated` event
  - `extend_dues(actor_id, member_id, months) -> Result<Member>` — validation + repo set_dues_paid_until_with_revival + audit + `MemberUpdated` event
  - `set_dues(actor_id, member_id, naive_date) -> Result<Member>` — same shape, set rather than extend
  - `expire_now(actor_id, member_id) -> Result<Member>` — repo expire + invalidate sessions + audit + `MemberUpdated` event
  - `update_discord_id(actor_id, member_id, discord_id) -> Result<Member>` — validate + repo + audit + `MemberUpdated` event
  - `resend_verification(actor_id, member_id) -> Result<()>` — token issue + email + audit
  - `create(actor_id, CreateMemberRequest) -> Result<Member>` — repo create + welcome email + audit + (no integration event — `MemberActivated` only fires on activate)
- **Welcome-email helper** (`send_welcome_email`) and `dispatch_member_updated` move from the handler module into `MemberService` as private methods.
- **AppState** gains a `member_service: Arc<MemberService>` field; `ServiceContext` constructs and exposes it.
- **`admin_refund_payment` is out of scope** — it lives in `members.rs` for routing reasons but operates on payments. Keeps its current shape until the next pass.
- **BREAKING (internal only)**: spec-level requirement flips for member operations — handlers no longer call `audit_service` / `integration_manager` directly for these paths. `admin-members`, `audit-logging`, and `integration-events` deltas reflect the move.

## Capabilities

### New Capabilities
- `member-admin-service`: Service that owns the full side-effect chain for admin-driven member mutations (repo update, session invalidation, audit log, integration dispatch, transactional emails). Single entrypoint for every admin action, so a forgotten step is structurally impossible.

### Modified Capabilities
- `admin-members`: Handler-level requirements change — handlers SHALL call `MemberService` rather than calling `member_repo` + `audit_service` + `integration_manager` directly. The wire shape (URLs, methods, HTMX fragments) is unchanged.
- `audit-logging`: "Locus of audit emission varies by domain" requirement updates — member operations join payments in emitting from the service layer. Settings, types, announcements, events still emit from handlers (out of scope for this change).
- `integration-events`: "Events are dispatched from handlers (not services) for member operations" requirement reverses — for member operations, dispatch moves into `MemberService`. Other domains unchanged.

## Impact

- **Code**: new file `src/service/member_service.rs` (~600 lines moved + reorganized, no net behavior change). `src/web/portal/admin/members.rs` shrinks substantially as inline orchestration moves out — roughly handler bodies of 50–80 lines drop to 15–25 each. `src/api/state.rs` and `src/service/mod.rs` gain `member_service` plumbing.
- **Wire shape**: zero change. Same URLs, same form bodies, same HTMX response fragments, same audit-log rows, same integration events. Pure internal refactor.
- **Tests**: existing handler-level tests should pass unmodified (they assert HTTP responses, not call structure). Add new unit tests for `MemberService` covering each method's full side-effect chain — this is the part that was previously untested at this granularity because it lived in handler code mixed with HTTP concerns.
- **CLAUDE.md alignment**: closes the gap between the documented rule and observed behavior for member operations. Sets the pattern for follow-up changes covering events, announcements, types, and settings.
- **Risk**: medium — touches every member-admin handler. Mitigation: tests at both layers; the wire-shape contract is the contract that matters and is unchanged.
