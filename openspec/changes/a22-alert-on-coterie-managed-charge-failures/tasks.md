## 1. Add the AdminAlert dispatch

- [ ] 1.1 In `src/service/billing_service/auto_renew.rs::process_scheduled_payment`, locate the `if sp.retry_count + 1 >= max_retries` branch (currently at ~line 653 — verify during implementation).
- [ ] 1.2 Inside that branch, after the existing `update_status(...).await?` and `tracing::warn!(...)`, add:
  ```rust
  // Re-fetch member for the alert body. Best-effort: if the lookup
  // fails (shouldn't, the member was just charged), log and skip.
  if let Ok(Some(member)) = self.member_repo.find_by_id(sp.member_id).await {
      let amount_display = format!("${:.2}", sp.amount_cents as f64 / 100.0);
      let dues_until = member.dues_paid_until
          .map(|d| d.format("%B %d, %Y").to_string())
          .unwrap_or_else(|| "(unknown)".to_string());
      let portal_url = format!(
          "{}/portal/admin/members/{}",
          self.base_url.trim_end_matches('/'),
          member.id,
      );
      let subject = format!(
          "Coterie-managed renewal failed (final) — {}",
          member.full_name,
      );
      let body = format!(
          "Member: {} <{}>\n\
           Amount: {}\n\
           Retry count: {} / {}\n\
           Last failure: {}\n\
           Member detail: {}\n\
           \n\
           The auto-renew has been marked Failed permanently. The member's \
           membership will lapse on {}. Operator action: contact the member \
           or invite them to update their payment method via the dues-\
           restoration flow.",
          member.full_name, member.email, amount_display,
          sp.retry_count + 1, max_retries, e,
          portal_url, dues_until,
      );
      self.integration_manager
          .handle_event(IntegrationEvent::AdminAlert { subject, body })
          .await;
  } else {
      tracing::warn!(
          "Couldn't re-fetch member {} after terminal scheduled-payment failure — \
           AdminAlert not dispatched. Logs are the only record of this failure.",
          sp.member_id,
      );
  }
  ```
- [ ] 1.3 Confirm `AutoRenew` holds `base_url: String` already (it should — `BillingService::new` passes it through to `Notifications` and likely also to `AutoRenew`). If not, add it as a constructor parameter and thread through from `BillingService::new`.
- [ ] 1.4 Confirm `IntegrationEvent` is already imported at the top of `auto_renew.rs`. If not, add `use crate::integrations::IntegrationEvent;`.

## 2. Tests

- [ ] 2.1 In `src/service/billing_service/auto_renew.rs::tests` (or wherever the existing auto_renew tests live), add `terminal_failure_dispatches_admin_alert`:
  - Seed a member with `dues_paid_until = now + 30d` (relative anchor per the `a14`/`a15` rule).
  - Create a scheduled_payment row for that member with `retry_count = max_retries - 1` (one more failure will trip terminal).
  - Configure the test `FakeStripeGateway` to return a charge failure (`PaymentIntentResult::RequiresAction` or whatever the existing test fixtures use for the failure case).
  - Call `process_scheduled_payment(scheduled_id).await`.
  - Assert: the scheduled_payment row is now `Failed`. AND: the test `IntegrationManager` recorded an `AdminAlert` event with the expected subject substring "Coterie-managed renewal failed (final)".
- [ ] 2.2 Add `transient_failure_does_not_dispatch_admin_alert`:
  - Same setup, but `retry_count = 0` (or any value below `max_retries - 1`).
  - Same charge-failure setup.
  - Call `process_scheduled_payment(scheduled_id).await`.
  - Assert: the scheduled_payment row is back to `Pending` (or whatever the retry path uses). AND: the test `IntegrationManager` recorded ZERO `AdminAlert` events.
- [ ] 2.3 If the test `IntegrationManager` doesn't record events in a way the test can assert on, extend it (the same shim discussed in `a21-test-stripe-subscription-webhook-paths` design D3). If `a21` lands first and adds the shim, this test reuses it.

## 3. Validate

- [ ] 3.1 `cargo build --all-targets --features test-utils` — clean.
- [ ] 3.2 `cargo test --features test-utils` — full suite passes including the two new tests.
- [ ] 3.3 Grep verify: `grep -nE "IntegrationEvent::AdminAlert" src/service/billing_service/auto_renew.rs` shows at least two dispatches — the one this change adds and the existing "couldn't schedule next renewal" alert.
