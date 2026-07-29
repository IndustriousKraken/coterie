# paid-events Specification

## MODIFIED Requirements

### Requirement: Pass pricing is flat, with no proration in either direction

A pass SHALL cost the full price regardless of how many sessions remain when it is
bought, and a refunded pass SHALL be refunded in full regardless of how many
sessions have already been held.

No per-session proration, partial refund, or make-up credit SHALL be computed.
This is a deliberate policy, recorded here so that a future contributor
implements the flat rule on purpose rather than treating its simplicity as an
oversight: per-session accounting introduces questions this capability declines to
answer (what a cancelled occurrence is worth, what a partially-attended class is
worth) that no organization has yet needed answered.

Flat pricing has a floor: a pass SHALL be sellable only while at least one
occurrence of the series has not yet started. Enrollment in a series whose every
occurrence has already started SHALL be rejected with a `BadRequest`, with no
enrollment, no payment, and no Checkout session created. "Not yet started" SHALL
be tested on each occurrence's derived UTC instant, not on its stored wall-clock,
so a non-UTC organization's evening session does not drop out of the future by the
organization's offset. Confirming such a pass would materialize attendance for
zero occurrences, which is money taken for a seat that does not exist — the
failure this capability exists to prevent — and it is a floor rather than a
proration rule, so a buyer with one session left still pays the full price.

The floor SHALL be enforced at the enrollment entry points a buyer reaches — the
member enrollment path and the public guest-enrollment path — so a bookmarked URL
or a direct POST is rejected the same way the class page's hidden form is. It
SHALL NOT block an administrator recording an at-the-door payment or comping an
enrollment, which already bypass the capacity guard on the same grounds, nor the
checkout-completion webhook settling a purchase that was started while a session
remained.

#### Scenario: A late enrollee pays full price

- **WHEN** someone enrolls in a six-session class with two sessions remaining
- **THEN** they SHALL be charged the full pass price

#### Scenario: A mid-class refund returns the full amount

- **WHEN** an admin refunds a pass after four of six sessions have been held
- **THEN** the full pass price SHALL be refunded, not a prorated remainder

#### Scenario: A finished class cannot be bought

- **WHEN** a member or a guest submits enrollment for a series whose every
  occurrence has already started
- **THEN** the call SHALL return `BadRequest`, and no `series_enrollment` row, no
  `SeriesPass` payment, and no Checkout session SHALL be created

#### Scenario: One remaining session is still a sale

- **WHEN** someone enrolls in a six-session class with five sessions already
  started and one still to come
- **THEN** the enrollment SHALL proceed at the full pass price, and a confirmed
  pass SHALL materialize attendance for that one remaining occurrence

#### Scenario: An admin can still comp a finished class

- **WHEN** an admin comps an enrollment or records an at-the-door pass for a
  series whose sessions have all started
- **THEN** the enrollment SHALL be recorded, because the administrative paths
  deliberately bypass this guard as they already bypass the capacity guard
