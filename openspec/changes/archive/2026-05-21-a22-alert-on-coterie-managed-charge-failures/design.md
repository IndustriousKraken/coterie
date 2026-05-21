## Context

The codebase has an established pattern for "operator needs to know about this": `IntegrationEvent::AdminAlert { subject, body }`. It routes through `IntegrationManager::handle_event(...)`, which fans out to:

- `AdminAlertEmailIntegration` — sends the alert to `org.contact_email`.
- Discord integration (if configured) — posts to the admin-alerts channel.
- Any future alert sinks.

This pattern is correctly used in:
- `notify_subscription_payment_failed` (Stripe-managed charge failure).
- `notify_subscription_cancelled` (Stripe sub deleted out-of-band).
- The webhook dispatcher's "couldn't resolve membership type for paid Checkout session" path.
- Refund handler in `payment_admin_service`.
- `process_scheduled_payment`'s "charged successfully but couldn't schedule next renewal" path.

The pattern is NOT used in: `process_scheduled_payment`'s "charge failed, max retries exhausted" path. That's the only operator-relevant terminal-failure path in the billing system without an alert. This change fixes that.

## Goals / Non-Goals

**Goals:**
- Permanent Coterie-managed charge failures fire an `AdminAlert`.
- The alert content includes everything an operator needs to triage: member identity, amount, retry count, last failure reason, and a deep link to the member's admin page.
- No alert spam on transient retry failures — only on terminal failure.

**Non-Goals:**
- Adding a member-facing email for Coterie-managed failures. The `notify_subscription_payment_failed` template is Stripe-specific (it says "Stripe charged your card"); a parallel Coterie-managed template is a separate concern.
- Changing retry counts or backoff intervals.
- Adding alerts elsewhere in the billing system.
- Adding an "AdminAlert summary digest" feature (multiple failures → one rolled-up email).

## Decisions

### D1. Alert only on terminal failure, not per-retry

Per-retry alerts would spam operators. A typical failed-card scenario fires retry 1, retry 2, retry 3, then terminal — four alerts for one underlying problem. By contrast, today's logs are quiet for transients and noisy only on terminal. Match that semantic with the alert.

If an operator wants per-retry visibility, they can tail the logs or watch the billing dashboard.

### D2. Alert subject and body content

```
Subject: Coterie-managed renewal failed (final) — Jane Smith

Body:
Member: Jane Smith <jane@example.com>
Amount: $50.00
Retry count: 3 / 3
Last failure: Card declined: insufficient_funds
Member detail: https://coterie.neontemple.com/portal/admin/members/<uuid>

The auto-renew has been marked Failed permanently. The member's
membership will lapse on <dues_paid_until>. Operator action: contact
the member or invite them to update their payment method via the
dues-restoration flow.
```

Match the existing `notify_subscription_payment_failed` body shape — multi-line plain-text, fields labeled, includes a "what to do next" hint. Operators reading the email or Discord post should get full context without clicking through.

### D3. Where to dispatch in the code

Inside `process_scheduled_payment`, in the existing branch:

```rust
if sp.retry_count + 1 >= max_retries {
    self.scheduled_payment_repo
        .update_status(id, ScheduledPaymentStatus::Failed, ...)
        .await?;
    tracing::warn!(...);

    // NEW: dispatch AdminAlert here
    // 1. Re-fetch the member for full_name / email / dues_paid_until
    // 2. Compute the portal URL (base_url + /portal/admin/members/<uuid>)
    // 3. Build subject + body
    // 4. self.integration_manager.handle_event(AdminAlert { subject, body }).await;
}
```

Re-fetch the member because the scheduled_payment row has `member_id` but not the rich fields. The fetch is best-effort — if it fails, log and skip the alert (don't fail the parent operation; the row is already marked Failed).

### D4. Base URL for the portal link

`AutoRenew` already holds `base_url` (set in its constructor via `BillingService::new`). Reuse it. The portal URL is `format!("{}/portal/admin/members/{}", base_url.trim_end_matches('/'), member.id)`.

### D5. Failure semantics inherit from existing AdminAlert dispatches

Per the established pattern: if `integration_manager.handle_event` "fails" (one or more integrations error), the failure is logged internally and the call returns. The parent `process_scheduled_payment` continues normally — the scheduled_payment row is already marked Failed.

### D6. Test covers both branches

The test for "terminal failure dispatches AdminAlert" needs to make `process_scheduled_payment` actually reach the terminal branch. That requires a scheduled-payment row with `retry_count` already at `max_retries - 1` (so the next failure ticks it over). Set up the test fixture accordingly.

The test for "transient failure does NOT dispatch AdminAlert" sets up `retry_count = 0` (or any value below max-1) and asserts the integration manager received NO `AdminAlert` after a single failure.

## Risks / Trade-offs

- **Risk**: alert spam if an org has dozens of stale Coterie-managed members with bad cards lined up. Each one fires when its terminal-retry attempts. → **Mitigation**: this is actually correct behavior — operator NEEDS to know about each one (they're separate members with separate billing problems). A future digest/throttling feature can roll them up if it becomes painful. Not a real risk for typical small-org scale.
- **Risk**: the alert email goes to `org.contact_email` which the operator forgets to set. → **Mitigation**: the `AdminAlertEmailIntegration` already logs at debug level when it skips for missing `org.contact_email`. Operators can verify it's set via `/portal/admin/settings`. Pre-deploy checklist item.
- **Trade-off**: dispatching from inside the scheduled-payment loop adds an integration-handle latency to every terminal failure. With at most ~5-10 terminal failures per day for a small org, the latency cost is negligible.

## Migration Plan

Single PR.

1. In `src/service/billing_service/auto_renew.rs::process_scheduled_payment`, locate the `if sp.retry_count + 1 >= max_retries` branch.
2. After the existing `update_status(...).await?;` and `tracing::warn!(...);`, add:
   - Re-fetch member: `let member = self.member_repo.find_by_id(sp.member_id).await.ok().flatten();`
   - If `Some(m)`:
     - Compute portal URL.
     - Build subject and body matching the design's content shape.
     - Call `self.integration_manager.handle_event(IntegrationEvent::AdminAlert { subject, body }).await;`
   - If `None` (shouldn't happen — member existed when we started the charge — but be defensive): log and skip the alert.
3. Add tests:
   - `terminal_failure_dispatches_admin_alert` — set retry_count to max-1, force a charge failure (via the FakeStripeGateway), confirm the AdminAlert was recorded in the IntegrationManager's events.
   - `transient_failure_does_not_dispatch_admin_alert` — set retry_count to 0, force a charge failure, confirm no AdminAlert was recorded.
4. `cargo build --all-targets --features test-utils` — clean.
5. `cargo test --features test-utils` — full suite passes.
