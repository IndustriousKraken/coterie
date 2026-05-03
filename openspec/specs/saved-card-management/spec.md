# saved-card-management Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Card lifecycle uses Stripe SetupIntent

Adding a saved card SHALL use Stripe's SetupIntent flow:
1. Member's portal page calls `POST /api/payments/cards/setup-intent` to create a SetupIntent.
2. Stripe.js confirms the SetupIntent in the browser.
3. The portal calls `POST /api/payments/cards` to record the now-confirmed payment method against the member.

The system SHALL NOT receive raw card numbers; only Stripe payment-method ids SHALL be persisted.

#### Scenario: SetupIntent creation requires authentication

- **WHEN** an anonymous request hits `POST /api/payments/cards/setup-intent`
- **THEN** the response SHALL be 401 Unauthorized

#### Scenario: Card record cannot be created without prior SetupIntent confirmation

- **WHEN** a portal POST attempts to record a card whose `pm_*` id was not produced by Stripe.js for this member
- **THEN** the recording call SHALL fail when Stripe rejects attaching the payment method

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

