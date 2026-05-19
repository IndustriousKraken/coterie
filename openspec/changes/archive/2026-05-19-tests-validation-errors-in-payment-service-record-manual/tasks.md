## 1. Add validation-error tests for PaymentService::record_manual

- [x] 1.1 `record_manual_rejects_negative_amount` — asserts
  `record_manual(amount_cents = -100)` returns
  `Err(AppError::BadRequest("amount_cents must not be negative"))`
  and that the `payments` table has no rows.
- [x] 1.2 `record_manual_rejects_over_cap_amount` — asserts
  `record_manual(amount_cents = MAX_PAYMENT_CENTS + 1)` returns
  `Err(AppError::BadRequest(_))` and the error message contains the
  dollar value of the cap.
- [x] 1.3 `record_manual_rejects_stripe_method` — asserts
  `record_manual(payment_method = PaymentMethod::Stripe)` returns
  `Err(AppError::BadRequest(_))` whose message references Stripe.
- [x] 1.4 `record_manual_rejects_unknown_member` — uses a random
  `Uuid::new_v4()` as `member_id`; asserts `Err(AppError::BadRequest(_))`
  whose message includes the id substring.
- [x] 1.5 `record_manual_rejects_donation_with_stale_campaign_id` —
  inserts a real member, passes `PaymentKind::Donation { campaign_id:
  Some(Uuid::new_v4()) }`; asserts `Err(AppError::BadRequest(_))` and
  that no `payments` row was created.

## 2. Add audit-action mapping tests for PaymentService::record_manual

- [x] 2.1 `record_manual_waived_dues_audits_as_waive_dues` — record
  `(PaymentMethod::Waived, PaymentKind::Membership)`; assert exactly
  one `audit_logs` row exists with `action = "waive_dues"`.
- [x] 2.2 `record_manual_cash_membership_audits_as_manual_payment` —
  record `(PaymentMethod::Cash, PaymentKind::Membership)`; assert an
  `audit_logs` row exists with `action = "manual_payment"`.
- [x] 2.3 `record_manual_donation_audits_as_manual_donation` — record
  `(PaymentMethod::Cash, PaymentKind::Donation { campaign_id: None })`;
  assert an `audit_logs` row exists with `action = "manual_donation"`.
