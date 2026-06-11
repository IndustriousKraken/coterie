## Why

When a Coterie-managed scheduled-payment charge fails, the only operator-visible signal today is a `tracing::warn!` log line. `src/service/billing_service/auto_renew.rs::process_scheduled_payment` writes:

```rust
tracing::warn!("Scheduled payment {} failed permanently for member {}: {}", id, sp.member_id, e);
```

…and that's it. No AdminAlert dispatch, no email, no Discord post. An operator only finds out about a Coterie-managed billing failure by:
- Tailing the application logs (requires SSH).
- Checking `/portal/admin/billing/dashboard`'s "recent failures" widget (requires manual visit).
- A member complaining their dues didn't renew.

By contrast, Stripe-managed billing failures already dispatch `AdminAlert` via `notify_subscription_payment_failed`. That alert routes through `AdminAlertEmailIntegration` (email to `org.contact_email`) and the Discord integration (if configured). The pattern is established; Coterie-managed failures just don't use it.

This is fine today because there are zero Coterie-managed members in production — the neontemple.com launch starts everyone in `StripeSubscription` mode. Coterie-managed members appear only as they organically migrate (months from launch). But the gap closes that window: by the time anyone's Coterie-managed, the alert is in place.

## What Changes

- **Dispatch `IntegrationEvent::AdminAlert` from `process_scheduled_payment`** on charge failure. Two dispatch points:
  - **Per-retry transient failure** (charge failed, retry queued): NO alert. Same shape as today's Stripe behavior — operators don't need a ping every time Stripe says "card declined, will retry." The member's notification (if any) is the right place for this; the operator only needs to know when it's terminal.
  - **Permanent failure** (max-retries exhausted, transitioning to `Failed` status): ALERT. Operator needs to know "this member's auto-renew is dead and they'll lapse unless they intervene."
- **Alert content**:
  - Subject: `Coterie-managed renewal failed (final) — <member.full_name>`
  - Body: member name + email + amount + retry-count + the last failure_reason from the row + a portal link to the member's admin page.
- **No changes to the retry mechanism or to the per-retry logging**. Only the alert dispatch is added.
- **Optional secondary change**: also alert on the "charged-but-couldn't-schedule-next-renewal" path that already exists at the bottom of `process_scheduled_payment` (line ~611). That path already dispatches an AdminAlert; verify the shape during implementation. No new dispatch needed if it's already correct.
- **Out of scope**:
  - Member-facing notification email for Coterie-managed failures. The card-declined email already exists for Stripe-managed; a future change can wire a parallel email for Coterie-managed. Out of scope here.
  - Changing the retry count or backoff schedule.
  - Alerting on per-retry failures.

## Capabilities

### New Capabilities

(None — extends existing capabilities.)

### Modified Capabilities
- `recurring-billing`: requires that permanent Coterie-managed charge failures dispatch `AdminAlert` — closing the parity gap with Stripe-managed failures.
- `integration-events`: documents that `AdminAlert` is dispatched from `AutoRenew::process_scheduled_payment` on terminal failure (joining the existing dispatch sites in `notify_subscription_payment_failed` and `notify_subscription_cancelled`).

## Impact

- **Code**: ~20 lines added to `src/service/billing_service/auto_renew.rs::process_scheduled_payment` — the AdminAlert dispatch inside the `if sp.retry_count + 1 >= max_retries` branch.
- **Wire shape**: zero member-facing change. Operators with Discord and/or `org.contact_email` configured start receiving alerts on terminal Coterie-managed failures.
- **Tests**:
  - Add a test asserting the AdminAlert is dispatched on max-retries-exhausted.
  - Add a test asserting NO AdminAlert is dispatched on a transient retry (so operators aren't spammed).
- **Risk**: very low. The dispatch is additive; the existing logic is untouched.
- **Production timing**: post-launch is acceptable. There are no Coterie-managed members on day one (everyone's imported in `StripeSubscription` mode). But it's small enough to ship pre-launch if there's time.
- **Dependency**: independent of every other queued change.
