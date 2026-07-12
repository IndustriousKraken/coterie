# saved-card-management Specification

## MODIFIED Requirements

### Requirement: Card lifecycle uses Stripe SetupIntent

Interactively adding a NEW saved card from the portal SHALL use Stripe's SetupIntent flow:

1. Member's portal page calls `POST /api/payments/cards/setup-intent` to create a SetupIntent.
2. Stripe.js confirms the SetupIntent in the browser.
3. The portal calls `POST /api/payments/cards` to record the now-confirmed payment method against the member.

These are the *only* two JSON endpoints under `/api/payments/cards/*`. List, removal, and default-flag-setting flows SHALL go through the HTML endpoints under `/portal/api/payments/cards/*` (see `member-saved-cards`).

The system SHALL NOT receive raw card numbers; only Stripe payment-method ids SHALL be persisted.

Card ATTACHMENT — getting a payment method onto the member's Stripe customer — SHALL happen only inside Stripe-hosted flows. Two are sanctioned: the SetupIntent flow above, and a Stripe Checkout session created with `payment_intent_data.setup_future_usage=off_session` bound to the member's customer (the pay-at-signup auto-renew enrollment; Stripe attaches the paying card as part of its own hosted payment page). After a Checkout-attached card, Coterie SHALL only MIRROR the already-attached payment method locally — reading `pm_*` ids and display metadata from Stripe, never raw card numbers, de-duplicating by card fingerprint, and marking the mirrored card default only when the member has no default. A one-time backfill/sync MAY likewise persist local card records that REFERENCE payment methods ALREADY attached to the member's Stripe customer; it SHALL receive no raw card numbers, SHALL de-duplicate by card fingerprint, and SHALL set the member's default from the Stripe customer's default payment method. Neither mirroring path is card creation and neither uses SetupIntent; both only reflect existing Stripe state.

#### Scenario: SetupIntent creation requires authentication

- **WHEN** an anonymous request hits `POST /api/payments/cards/setup-intent`
- **THEN** the response SHALL be 401 Unauthorized

#### Scenario: Card record cannot be created without prior SetupIntent confirmation

- **WHEN** a portal POST attempts to record a card whose `pm_*` id was not produced by Stripe.js for this member
- **THEN** the recording call SHALL fail when Stripe rejects attaching the payment method

#### Scenario: List/delete/set-default are not under /api/*

- **WHEN** any caller looks for a JSON listing, removal, or default-flag-setting endpoint under `/api/payments/cards/*`
- **THEN** none exists; those flows live under `/portal/api/payments/cards/*` and return HTML fragments

#### Scenario: Backfill imports references to already-attached Stripe cards without SetupIntent

- **WHEN** the one-time backfill runs for a member whose Stripe customer already has attached cards
- **THEN** it SHALL persist a local record referencing each existing `pm_*` id (no SetupIntent, no raw card number), skipping any whose fingerprint already exists, and SHALL set the default from the Stripe customer's default payment method

#### Scenario: Signup-checkout card is mirrored, not created

- **WHEN** a pay-at-signup Checkout session created with `setup_future_usage=off_session` completes and enrollment runs
- **THEN** Coterie SHALL persist a local record referencing the payment method Stripe attached during Checkout (`pm_*` id + display metadata only), skipping any whose fingerprint already exists, and SHALL mark it default only when the member has no default card
