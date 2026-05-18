## ADDED Requirements

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
