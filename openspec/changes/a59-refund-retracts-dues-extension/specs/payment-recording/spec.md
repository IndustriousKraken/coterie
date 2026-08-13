# payment-recording Specification Delta

## ADDED Requirements

### Requirement: Refunding a membership payment retracts the dues it granted

A refunded membership payment SHALL retract the dues extension that payment
granted, so a member does not retain membership whose fee has been returned to
them. This SHALL hold whether the refund is issued through the admin refund route
or observed out-of-band via a `charge.refunded` webhook, matching how a refunded
event fee already releases its seat and a refunded class pass already cancels its
enrollment.

The rule across payment kinds is that a refund undoes what its payment bought.
Membership was the one kind that did not follow it, with the result that a
refunded payment left `dues_paid_until` and `dues_extended_at` in place and every
consumer of the dues window — the renewal runner, the member's status, the portal
— continued to read the member as paid. A refund produced a renewal charge a
month later on the production instance for exactly this reason.

The retraction SHALL reverse the extension that payment applied, not reset the
dues window to a fixed point. A member may hold dues granted by several payments,
and refunding one SHALL undo that one.

A payment that granted no dues extension SHALL have nothing retracted, and
retraction SHALL be safe to apply to a payment already retracted. Refund handling
already tolerates echoes of itself, and this SHALL NOT introduce an operation
that behaves differently the second time it runs.

Retraction SHALL be recorded in the audit log, naming the payment that caused it,
as the event-seat and enrollment retractions already are. A member's dues window
moving without a traceable cause is not acceptable on a financial record.

Retraction SHALL NOT write a member's status directly. Where the retracted window
leaves a member no longer paid up, the existing status transition SHALL handle
that, so there remains one path by which a member becomes Expired.

Partial refunds SHALL remain excluded from automatic retraction, keeping the
existing behavior of leaving the row untouched and alerting an operator. The
existing reason stands — a partially refunded term does not correspond to an
obvious partial membership — but the alert SHALL state that the dues window was
not adjusted, so an operator does not assume it was handled.

#### Scenario: Admin refund retracts the dues extension

- **WHEN** an admin refunds a `Completed` membership payment that extended dues
- **THEN** the payment SHALL become `Refunded` and the member's dues window SHALL
  be reduced by the extension that payment granted

#### Scenario: Out-of-band Stripe refund retracts it too

- **WHEN** a `charge.refunded` event arrives for a membership payment refunded
  from the Stripe dashboard
- **THEN** the dues extension SHALL be retracted exactly as on the admin path

#### Scenario: A refunded membership does not renew

- **WHEN** a membership payment is refunded and the renewal runner subsequently
  evaluates that member
- **THEN** the member SHALL NOT be charged a renewal on the strength of the
  refunded payment's dues window

#### Scenario: A payment that granted no dues retracts nothing

- **WHEN** a refunded membership payment has no recorded dues extension
- **THEN** the member's dues window SHALL be unchanged and no error SHALL be
  raised

#### Scenario: Retraction is idempotent

- **WHEN** retraction runs twice for the same payment — an admin refund followed
  by its own webhook echo
- **THEN** the dues window SHALL be reduced once

#### Scenario: Retraction is audited

- **WHEN** a dues extension is retracted
- **THEN** an audit entry SHALL record it and name the payment that caused it

#### Scenario: A partial refund says the dues window was not adjusted

- **WHEN** a partial refund is observed on a membership payment
- **THEN** the payment row SHALL be left as-is and the operator alert SHALL state
  that the dues window was not adjusted
