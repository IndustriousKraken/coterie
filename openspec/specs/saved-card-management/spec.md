# saved-card-management Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Card lifecycle uses Stripe SetupIntent

Adding a saved card SHALL use Stripe's SetupIntent flow:

1. Member's portal page calls `POST /api/payments/cards/setup-intent` to create a SetupIntent.
2. Stripe.js confirms the SetupIntent in the browser.
3. The portal calls `POST /api/payments/cards` to record the now-confirmed payment method against the member.

These are the *only* two JSON endpoints under `/api/payments/cards/*`. List, removal, and default-flag-setting flows SHALL go through the HTML endpoints under `/portal/api/payments/cards/*` (see `member-saved-cards`).

The system SHALL NOT receive raw card numbers; only Stripe payment-method ids SHALL be persisted.

#### Scenario: SetupIntent creation requires authentication

- **WHEN** an anonymous request hits `POST /api/payments/cards/setup-intent`
- **THEN** the response SHALL be 401 Unauthorized

#### Scenario: Card record cannot be created without prior SetupIntent confirmation

- **WHEN** a portal POST attempts to record a card whose `pm_*` id was not produced by Stripe.js for this member
- **THEN** the recording call SHALL fail when Stripe rejects attaching the payment method

#### Scenario: List/delete/set-default are not under /api/*

- **WHEN** any caller looks for a JSON listing, removal, or default-flag-setting endpoint under `/api/payments/cards/*`
- **THEN** none exists; those flows live under `/portal/api/payments/cards/*` and return HTML fragments

### Requirement: Default-card invariant maintained

A member SHALL have at most one default card at any time. Setting a card as default SHALL clear the previous default in the same transaction.

#### Scenario: Setting a new default unsets the previous

- **WHEN** a member with a default card sets another card as default
- **THEN** at the end of the transaction exactly one card SHALL be marked default

### Requirement: Card data persists only the necessary metadata

The saved-card row SHALL persist only:
- Stripe payment-method id (`pm_*`).
- Brand and last-four (for UI display).
- Expiry month/year (for "expiring soon" UI hints).
- Member id (owner).
- Default flag.
- Created-at timestamp.

The full card number, CVV, and full expiry SHALL never be persisted.

#### Scenario: Database row contains no PAN/CVV

- **WHEN** a saved-card row is inspected
- **THEN** it SHALL contain no PAN, no CVV, and only the metadata listed above

### Requirement: Test fixtures representing valid saved cards use runtime-relative expiry dates

Test fixtures (in particular `FakeStripeGateway`'s default `retrieve_payment_method` response) that represent "a valid, non-expired saved card" SHALL use a `exp_year` computed from `Utc::now().year()` at runtime, NOT a hardcoded future year literal.

The reason: a hardcoded future year is a time-bomb — it represents a valid card today but becomes an expired card once wall-clock time reaches that year. Tests that implicitly depend on the default-fixture-card being valid will silently break. The same anti-pattern is addressed in the `admin-events` spec for materializer test anchors.

Tests that deliberately exercise expired-card behavior MAY use hardcoded past or near-future expiry dates as inputs — the rule applies to fixtures representing "valid" cards, not to fixtures representing specific calendar boundary cases.

#### Scenario: FakeStripeGateway default fixture is valid for years to come

- **WHEN** a test calls `retrieve_payment_method` without queuing a specific response
- **THEN** the returned `PaymentMethodDetails.exp_year` SHALL be at least `Utc::now().year() + 1` (i.e., never representing an expired card from the perspective of the running test)

#### Scenario: Hardcoded future year in a "valid card" fixture is a defect

- **WHEN** a contributor inspects a test fixture that represents "a valid saved card"
- **THEN** a literal future year (e.g., `exp_year: 2030`) SHALL be treated as a defect to be replaced with a runtime-relative computation; the rule is "fixtures representing validity SHALL be valid regardless of when the test runs"

