# payment-recording Specification

## MODIFIED Requirements

### Requirement: PaymentService::record_manual emits the audit-log entry

`record_manual` SHALL emit an audit-log entry via `audit_service.log` after a
successful repo write, using a centralized `audit_action(method, kind)`
mapping that produces the action string. The mapping SHALL be, in order:

- `(Waived, EventFee { .. })` → `"waive_event_fee"`
- `(Waived, SeriesPass { .. })` → `"waive_series_pass"`
- `(Waived, _)` → `"waive_dues"`
- `(_, Membership)` → `"manual_payment"`
- `(_, Donation { .. })` → `"manual_donation"`
- `(_, EventFee { .. })` → `"manual_event_fee"`
- `(_, SeriesPass { .. })` → `"manual_series_pass"`
- `(_, Other)` → `"manual_other"`

Every waived arm for a specific paid-events kind SHALL precede the general
`(Waived, _)` arm as shown, so a comped class audits as `"waive_series_pass"`
rather than being absorbed by the dues-waiver arm. Every arm that existed before
a given kind was added SHALL keep producing the same action string it produced
before, so existing audit history and any queries over it remain meaningful.

Centralization SHALL prevent the four sites that previously duplicated this
from drifting.

#### Scenario: Cash dues payment audits as manual_payment

- **WHEN** `record_manual` records a `(PaymentMethod::Manual,
  PaymentKind::Membership)` payment
- **THEN** the emitted audit row SHALL have `action = "manual_payment"`

#### Scenario: Waived dues audits as waive_dues

- **WHEN** `record_manual` records a `(PaymentMethod::Waived,
  PaymentKind::Membership)` payment
- **THEN** the emitted audit row SHALL have `action = "waive_dues"`

#### Scenario: Cash donation audits as manual_donation

- **WHEN** `record_manual` records a `(PaymentMethod::Manual,
  PaymentKind::Donation { .. })` payment
- **THEN** the emitted audit row SHALL have `action = "manual_donation"`

#### Scenario: At-the-door event payment audits as manual_event_fee

- **WHEN** `record_manual` records a `(PaymentMethod::Manual,
  PaymentKind::EventFee { .. })` payment
- **THEN** the emitted audit row SHALL have `action = "manual_event_fee"`

#### Scenario: Comped event seat audits as waive_event_fee, not waive_dues

- **WHEN** `record_manual` records a `(PaymentMethod::Waived,
  PaymentKind::EventFee { .. })` payment
- **THEN** the emitted audit row SHALL have `action = "waive_event_fee"`; it
  SHALL NOT be absorbed by the `(Waived, _)` arm and audited as `"waive_dues"`

#### Scenario: At-the-door class payment audits as manual_series_pass

- **WHEN** `record_manual` records a `(PaymentMethod::Manual,
  PaymentKind::SeriesPass { .. })` payment
- **THEN** the emitted audit row SHALL have `action = "manual_series_pass"`

#### Scenario: Comped class enrollment audits as waive_series_pass, not waive_dues

- **WHEN** `record_manual` records a `(PaymentMethod::Waived,
  PaymentKind::SeriesPass { .. })` payment
- **THEN** the emitted audit row SHALL have `action = "waive_series_pass"`; it
  SHALL NOT be absorbed by the `(Waived, _)` arm and audited as `"waive_dues"`

### Requirement: Payment-recording entry points are explicitly enumerated

Payments SHALL be recorded via exactly four entry points:

- **`PaymentService::record_manual`** — for non-Stripe payments (method `Manual` or `Waived` — cash, check, and in-kind are all recorded as `Manual` with detail in the description). Operator-initiated via the admin UI. The service SHALL reject `PaymentMethod::Stripe` with a `BadRequest`; Stripe payments SHALL go through one of the other entry points.
- **`WebhookDispatcher::handle_*`** — for Stripe-initiated events: customer paid an invoice, customer completed a checkout session, payment-intent succeeded. Inbound to Coterie, verified-signature, idempotency-claimed, dispatched per event type.
- **`BillingService::process_scheduled_payment`** — for Coterie-initiated auto-renew charges against a saved card. The scheduled payment row is the Coterie-side trigger; the Stripe charge is a direct API call (not a webhook); on charge success, the `Payment` row is created from the charge result.
- **Stripe payment-history backfill** — a one-time, idempotent, INSERT-only import of a member's historical Stripe charges/invoices that settled before Coterie was recording them. It persists via `payment_repo.create`, keyed on the Stripe id so a re-run creates nothing new, and — like the member CSV import — it emits its OWN audit rows (`import_payment` per row, `import_payments_batch` aggregate) rather than routing through the other three sites. It records only already-settled historical charges: it SHALL NOT initiate a charge, observe a live event, or extend dues.

All four entry points SHALL persist via `payment_repo.create(...)`. Direct `payment_repo.create` calls from handlers or services OTHER than these four SHALL be forbidden. Adding a fifth entry point requires updating this spec.

Why four, not three: `BillingService::process_scheduled_payment` doesn't fit `record_manual` (not operator-initiated) or the webhook path (no inbound event); and the historical backfill fits none of the three — it is neither operator-initiated, nor a live inbound event, nor a Coterie-initiated charge, but a bulk import of settled history. Like the member CSV import, it is INSERT-only, idempotent, and self-auditing, so it does not skip the audit trail the three-site rule exists to protect.

**Pending placeholder rows are a distinct act from recording a payment.** The four entry points above govern **recording a payment** — writing or settling a row that represents money actually collected. Separately, a flow that *initiates* a Stripe charge SHALL be permitted to write a `Pending` placeholder row at initiation time, before any money has moved, so the eventual webhook can find the row by its Stripe id and settle it. This is pre-existing behavior, not a new allowance: the membership-checkout, donation-checkout, and saved-card donation flows all write such a row today at session/charge creation. Event-fee registration and series-pass enrollment write one on the same basis, whether initiated by a logged-in member or by an anonymous guest.

A `Pending` placeholder SHALL NOT extend dues, dispatch integration events, or emit a payment audit row — it represents intent, not receipt. It SHALL become a recorded payment only by being settled through one of the four entry points above (in practice the webhook dispatcher), which is where the audit row and side effects fire. Because a placeholder carries no money and no side effects, permitting it does not reopen the audit-skipping hole the four-entry-point rule exists to close.

#### Scenario: record_manual rejects Stripe method

- **WHEN** a caller invokes `PaymentService::record_manual` with `PaymentMethod::Stripe`
- **THEN** the call SHALL return `BadRequest("Stripe payments are recorded via StripeClient, not record_manual")`

#### Scenario: Webhook handler is the only writer for Stripe-inbound events

- **WHEN** a Stripe payment-succeeded webhook event arrives
- **THEN** the webhook dispatcher's per-type handler SHALL construct the `Payment` value and call `payment_repo.create`; no other code path SHALL write payments from Stripe-inbound events

#### Scenario: Auto-renew charges write payments via BillingService

- **WHEN** a scheduled payment is processed and the saved-card charge succeeds
- **THEN** `BillingService::process_scheduled_payment` SHALL construct the `Payment` value and call `payment_repo.create`; the resulting payment row SHALL be linked to the scheduled-payment row and audited

#### Scenario: The historical backfill is the fourth entry point and self-audits

- **WHEN** the Stripe payment-history backfill imports a settled historical charge
- **THEN** it SHALL create the `Payment` row via `payment_repo.create` idempotently keyed on the Stripe id, SHALL emit its own `import_payment` audit row (and one `import_payments_batch` aggregate for the run), and SHALL NOT extend dues or dispatch integration events

#### Scenario: A fifth entry point requires a spec amendment

- **WHEN** a contributor adds a new code path that records a payment outside the four listed entry points
- **THEN** the PR SHALL be rejected pending an amendment to this requirement listing the new entry point; the rule exists to prevent accidental audit/event-skipping by ad-hoc payment-row writers

#### Scenario: An event-fee checkout opens a pending placeholder, not a recorded payment

- **WHEN** a member registers for a paid event and the event-fee Checkout session is created
- **THEN** a `Pending` payment row MAY be written at that moment without being a fifth entry point; it SHALL NOT extend dues, dispatch integration events, or emit a payment audit row, and it SHALL become a recorded payment only when the checkout-completion webhook settles it

#### Scenario: An abandoned placeholder never becomes a recorded payment

- **WHEN** a `Pending` placeholder's checkout session expires without payment
- **THEN** the row SHALL be flipped to `Failed` and SHALL never have counted as a recorded payment; no audit row for a collected payment SHALL have been emitted for it
