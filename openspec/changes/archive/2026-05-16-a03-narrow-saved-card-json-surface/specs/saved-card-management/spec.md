## MODIFIED Requirements

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
