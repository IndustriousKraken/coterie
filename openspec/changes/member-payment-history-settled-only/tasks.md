# Tasks

Member view only. The admin payment list MUST keep showing every status.

## 1. Filter the member list

- [ ] 1.1 In the member payment list handler (`src/web/portal/payments.rs`, the
  `/portal/payments` page + HTMX list fragment), filter the payments to
  `Completed` and `Refunded` before building `MemberPaymentRow`s. Do NOT change
  `admin/partials.rs` / the admin list.
- [ ] 1.2 Confirm receipts (`payments/receipts.rs`) and the annual statement are
  reached independently of this list filter (they key off settled/`paid_at`), so
  a member can still open a receipt for a Completed payment.

## 2. Tests

- [ ] 2.1 A member with `Completed`, `Refunded`, `Pending`, and `Failed` payments
  sees only the `Completed` and `Refunded` ones on `/portal/payments`.
- [ ] 2.2 An admin viewing that same member's payments still sees all four
  statuses.
- [ ] 2.3 Receipt/statement access for a Completed payment is unchanged.

## 3. Verify

- [ ] 3.1 `openspec validate member-payment-history-settled-only --strict` passes.
- [ ] 3.2 `cargo test` (payment/portal suites) green; `cargo clippy` clean.
