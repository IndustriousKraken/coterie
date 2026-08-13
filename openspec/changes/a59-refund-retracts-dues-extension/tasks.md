# Tasks

## 1. Retraction

- [ ] 1.1 Add a retraction operation that reverses the dues extension a specific
  membership payment granted, alongside the existing extension call in
  `BillingService`. It reverses **that payment's** extension, not a reset to a
  fixed date — a member may hold dues from several payments.
- [ ] 1.2 A payment with no recorded dues extension retracts nothing and is not an
  error.
- [ ] 1.3 Make it idempotent. The admin refund path and its own webhook echo can
  both reach it, and the existing refund handler already returns early on an echo
  — retraction must not depend on that early return being the only guard.
- [ ] 1.4 Do not write member status. Where the retracted window leaves a member
  no longer paid up, the existing transition handles it, so there stays one path
  by which a member becomes Expired.

## 2. Wire both refund paths

- [ ] 2.1 `src/payments/webhook_dispatcher/charge.rs::handle_charge_refunded` —
  add the membership branch beside the existing event-fee and series-pass
  branches, after `mark_refunded`. The two neighbouring branches are the model,
  including their audit-log calls.
- [ ] 2.2 The admin refund route — same retraction, so a refund from the admin UI
  and one from the Stripe dashboard have identical effect. The event-seat rule
  already holds on both paths; this must too.
- [ ] 2.3 Audit the retraction naming the causing payment, matching
  `event_registration_refunded` and `series_enrollment_refunded` in shape.

## 3. Partial refunds

- [ ] 3.1 Leave the behavior as-is: row untouched, `AdminAlert` raised.
- [ ] 3.2 Extend that alert's body to state that the dues window was **not**
  adjusted and that it is the operator's to correct. The current text explains the
  payment row is unchanged but leaves an operator to infer the dues consequence,
  which is the part that actually affects the member.

## 4. Tests

- [ ] 4.1 Admin refund of a membership payment reduces the member's dues window by
  that payment's extension.
- [ ] 4.2 Out-of-band `charge.refunded` for a membership payment does the same.
- [ ] 4.3 The renewal runner does not charge a member whose only qualifying dues
  came from a refunded payment. This is the production symptom — a refund on
  2026-07-11 produced a renewal charge on 2026-08-11 — so assert the end-to-end
  behavior, not only the retraction call.
- [ ] 4.4 A refunded membership payment that never extended dues leaves the window
  unchanged and raises no error.
- [ ] 4.5 Retraction applied twice reduces the window once.
- [ ] 4.6 A member holding dues from two payments, one refunded, retains the
  extension from the other.
- [ ] 4.7 Refunding an event fee and a class pass still behave exactly as they do
  today — this change adds a third branch and must not disturb the two that
  already work.
- [ ] 4.8 A partial refund leaves the row and the dues window untouched and the
  alert states so.
