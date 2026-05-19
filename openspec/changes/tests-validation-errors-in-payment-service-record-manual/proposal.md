## Why

`PaymentService::record_manual` (`src/service/payment_service.rs:74`) is the
single entry point for non-Stripe payment writes (cash, check, waived, other)
and also emits the audit-log row for those writes. It has **five distinct
validation error paths** at the top of the body:

1. `amount_cents < 0` → `BadRequest("amount_cents must not be negative")`
2. `amount_cents > MAX_PAYMENT_CENTS` → `BadRequest(...cap exceeded...)`
3. `payment_method == PaymentMethod::Stripe` → `BadRequest("Stripe payments are recorded via StripeClient...")`
4. Member not found → `BadRequest("member <id> not found")`
5. Donation with stale campaign id → `BadRequest("donation_campaign_id doesn't match...")`

These branches are also called out as `#### Scenario:` entries in
`openspec/specs/payment-recording/spec.md` (the "Validation at the service
boundary" requirement plus the "rejects Stripe method" scenario), but
`grep -l "PaymentService\|record_manual" tests/*.rs` returns **zero** matches.
Every validation guard exists in code with no test verifying it actually
fires — a regression that silently dropped one of these `if` blocks would
not be caught.

The `audit_action` helper at `src/service/payment_service.rs:179` is also
untested. Its 4-arm match maps `(method, kind)` to the action string; a
contributor could swap the (Waived, _) and (_, Membership) arms and the
suite would stay green.

## What Changes

Add a new integration-test file `tests/payment_service_test.rs` that
constructs a `PaymentService` against an in-memory SQLite + migrations
harness (following the pattern in `tests/event_reminder_test.rs` and
`tests/totp_test.rs`) and tests every validation branch plus the audit-
action mapping for the non-trivial happy paths.

New tests:

- `record_manual_rejects_negative_amount` — asserts `BadRequest` for
  `amount_cents = -100`; asserts no row in `payments`.
- `record_manual_rejects_over_cap_amount` — asserts `BadRequest` whose
  message names the cap (`MAX_PAYMENT_CENTS + 1`).
- `record_manual_rejects_stripe_method` — asserts the rejection message
  mentions Stripe / StripeClient.
- `record_manual_rejects_unknown_member` — uses a random `Uuid::new_v4()`
  as `member_id`; asserts `BadRequest` whose message includes the id.
- `record_manual_rejects_donation_with_stale_campaign_id` — inserts a
  member, passes a `PaymentKind::Donation { campaign_id: Some(random) }`;
  asserts no donation row is created.
- `record_manual_waived_dues_audits_as_waive_dues` — `(Waived, Membership)`
  should emit an `audit_logs` row whose `action = "waive_dues"`.
- `record_manual_cash_membership_audits_as_manual_payment` —
  `(Cash, Membership)` emits `action = "manual_payment"`.
- `record_manual_donation_audits_as_manual_donation` —
  `(Cash, Donation { .. })` emits `action = "manual_donation"`.

## Impact

- New file: `tests/payment_service_test.rs`.
- No production-code changes.
- Existing scenarios in `openspec/specs/payment-recording/spec.md` gain a
  fully-realized test counterpart; this change adds a couple of explicit
  audit-action scenarios under MODIFIED Requirements (see
  `specs/payment-recording/spec.md` in this change).
