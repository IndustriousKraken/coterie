## Why

The architecture pass's drift audit surfaced three specs that have fallen out of sync with the code. None of these are "code is wrong" findings — they're "spec is wrong." The code is correct per current architecture; the specs were written before the relevant refactors and never updated.

The three drifts:

1. **`integration-events`** has a requirement titled "Events are dispatched from handlers (not services) for member operations." It explicitly says "There is no `MemberService` wrapping these calls." But `MemberService` exists (`src/service/member_service.rs`) and dispatches integration events from inside the service (`member_service.rs:163-164`). The spec was written before `MemberService` was introduced.

2. **`audit-logging`** has a requirement titled "Locus of audit emission varies by domain" listing "Member operations" as handler-emitted. But `MemberService` emits audit logs from inside the service (`member_service.rs:149-157`). Same pre-refactor world.

3. **`payment-recording`** has a requirement titled "Two payment-recording entry points: PaymentService::record_manual and WebhookDispatcher" with the strict rule "Direct `payment_repo.create` calls from handlers or services other than these two SHALL be forbidden." But `BillingService::process_scheduled_payment` (auto-renewal of saved cards) is a third entry point that calls `payment_repo.create` directly. It's a legitimate third path (Coterie-initiated charge, no webhook involved), not a violation that should be refactored away.

Fixing these is purely spec text. Zero code change.

## What Changes

- **MODIFY `integration-events`** requirement to say that member-mutation events are dispatched from `MemberService` (not handlers), aligning with the CLAUDE.md "side-effects in services" rule.
- **MODIFY `audit-logging`** requirement to say member-operation audit emission is in `MemberService`. Update the inventory of locus-by-domain accordingly. Update the dependent scenarios.
- **MODIFY `payment-recording`** requirement to list three entry points: `PaymentService::record_manual`, `WebhookDispatcher::handle_*`, and `BillingService::process_scheduled_payment`. Add a brief explanation of why the third exists (Coterie-initiated charges have no webhook trigger).

No source code changes. No test changes. This is a documentation-shaped change.

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `integration-events` — corrected to reflect service-side dispatch for member operations.
- `audit-logging` — corrected to reflect service-side emission for member operations.
- `payment-recording` — corrected to acknowledge three entry points, not two.

## Impact

- **Code**: zero changes.
- **Wire shape**: zero changes.
- **Tests**: zero changes; no test currently asserts the inaccurate spec language.
- **Risk**: zero — text-only changes to documentation that already disagreed with the code.
- **Dependency**: none. Independent of all other queued changes.
