## Context

The `openspec/specs/` tree was bootstrapped via a `document-existing-architecture` change at some point in the past. That change captured the codebase's behavior at a point in time. Since then, three refactors changed reality without anyone updating the specs:

1. `MemberService` was introduced, moving audit emission and integration-event dispatch for member operations out of handlers and into the service. The CLAUDE.md architecture rule ("side-effects in services so handlers can't accidentally skip them") was applied to member ops; the specs documenting "audit/events in handlers" became stale.
2. `BillingService::process_scheduled_payment` was added for auto-renewal flows. It creates `Payment` rows directly (Coterie-initiated charge → success → record) without going through `PaymentService::record_manual` or `WebhookDispatcher`. This added a legitimate third entry point that the original two-entry-point spec didn't anticipate.

This change brings the specs back into sync with reality. No code moves. No behavior changes.

## Goals / Non-Goals

**Goals:**
- Each affected spec accurately describes what the code does today.
- Scenarios remain testable — every requirement keeps at least one scenario, and the scenarios match real code behavior.
- The "rule" framing of each requirement (what's forbidden, what's required) is preserved or strengthened — we're not loosening anything, just stating it accurately.

**Non-Goals:**
- Code changes. The code is correct; the specs were wrong.
- Refactoring `BillingService::process_scheduled_payment` to go through one of the existing two entry points. Auto-renew is genuinely a third shape (Coterie-initiated, no inbound webhook); funneling through a "fake webhook" or "fake manual record" would obscure the architecture.

## Decisions

### D1. integration-events: rewrite the member-ops requirement

Current spec text says events for member operations are dispatched from handlers, with the explicit phrase "There is no `MemberService` wrapping these calls."

New spec text: "Events for member operations are dispatched from `MemberService`." Scenarios update to assert that adding a new member-mutation method to `MemberService` must explicitly call `integration_manager.handle_event(...)`.

Strengthening note: now that side-effects are in the service, the rule that "side-effects in services so handlers can't skip them" applies in both directions — payments AND member ops. Updated text reflects this.

### D2. audit-logging: update the locus-by-domain inventory

Current inventory lists "Member operations" as handler-emitted. Updated to list as `MemberService`-emitted.

The "Settings, types, announcements, events" entry stays as handler-emitted — but a32 (separate change) is fixing the `types` part. After a32 ships, types will also be handler-emitted with actual audit calls (currently the spec says they audit but the code doesn't — a real bug, not just spec drift). a31 doesn't touch this; a32 handles it.

"Logout" stays as handler-emitted (`src/api/handlers/auth.rs`).

The overarching observation about CLAUDE.md being "aspirational" can be softened to: "Member operations and payments follow it; type/setting/announcement/event ops have audit in handlers."

### D3. payment-recording: rewrite the entry-points requirement

Current spec lists exactly two entry points and forbids direct `payment_repo.create` anywhere else.

New text lists three entry points:

1. `PaymentService::record_manual` — non-Stripe payments (Cash, Check, Waived, Other). Operator-initiated via the admin UI.
2. `WebhookDispatcher::handle_*` — Stripe-initiated events (customer paid an invoice, Stripe processed a checkout session, etc.). Inbound to Coterie.
3. `BillingService::process_scheduled_payment` — Coterie-initiated auto-renew charges against a saved card. The scheduled payment is the Coterie-side trigger; the Stripe charge is a direct API call (not a webhook); on success, the `Payment` row is created from the charge result.

The strict "forbidden anywhere else" rule stays — it just lists three, not two. Adding a fourth entry point would require its own spec amendment.

### D4. Scenario updates

Each modified requirement keeps at least one scenario. Stale scenarios are updated to match the new requirement text. New scenarios may be added where the change introduces a distinct testable case.

## Risks / Trade-offs

- **Risk**: a future contributor reads the updated spec and thinks "wait, the original said the opposite — was there a refactor?" → Mitigation: not a real risk. The OpenSpec system stores prior versions in archived changes; the rewrite is the new source of truth.
- **Risk**: tests written against the stale spec language fail after this change. → Mitigation: no tests currently assert "MemberService doesn't exist" or "exactly two entry points." Verified via grep.
- **Trade-off**: spec text gets longer (three entry points takes more words than two). Acceptable.

## Migration Plan

Single PR.

1. Rewrite the three modified requirements per D1/D2/D3, copying the FULL requirement block (text + every scenario) into the change's `specs/<capability>/spec.md` under `## MODIFIED Requirements`.
2. Verify each spec file's MODIFIED block matches the existing requirement's header exactly (whitespace-insensitive) so OpenSpec's archive step finds and replaces correctly.
3. `openspec validate a31-update-stale-specs` — confirms the structural changes are well-formed.
4. PR description explains: "spec text now matches what the code does. No code change."
