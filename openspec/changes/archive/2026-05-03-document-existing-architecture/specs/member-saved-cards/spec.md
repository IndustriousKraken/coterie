## ADDED Requirements

### Requirement: Saved-card management uses /api/* JSON for Stripe.js

Saved-card management for the member's own cards SHALL be exposed under `/api/payments/cards` because Stripe.js calls these endpoints directly via `fetch()`. The endpoints SHALL be:

- `GET /api/payments/cards` — list the authenticated member's saved cards.
- `POST /api/payments/cards/setup-intent` — create a Stripe SetupIntent.
- `POST /api/payments/cards` — record a saved card after Stripe confirms the SetupIntent.
- `PUT /api/payments/cards/:card_id/default` — set default card.
- `DELETE /api/payments/cards/:card_id` — remove a saved card.

These endpoints SHALL be gated by `require_auth` (member-self only). They are the only legitimate JSON endpoints under `/api/*` outside the Stripe webhook.

#### Scenario: Anonymous request returns 401

- **WHEN** an anonymous request hits `GET /api/payments/cards`
- **THEN** the response SHALL be 401 Unauthorized

#### Scenario: Member sees only their own cards

- **WHEN** a member fetches `/api/payments/cards`
- **THEN** the response SHALL include only cards owned by that member

#### Scenario: Stripe.js fetches use the X-CSRF-Token header

- **WHEN** Stripe.js calls `fetch('/api/payments/cards/setup-intent')` from a portal page
- **THEN** the request SHALL include `X-CSRF-Token` from the rendered `<meta name="csrf-token">` tag, validated by the top-level CSRF layer

### Requirement: Card removal clears default if needed

Removing the member's only card, or removing the card currently marked default, SHALL update the member's default-card state to a sensible value (no default if no cards remain; first remaining card otherwise) atomically.

#### Scenario: Removing the default card promotes another

- **WHEN** a member with two cards removes the one marked default
- **THEN** the remaining card SHALL be marked default in the same transaction

#### Scenario: Removing the only card leaves no default

- **WHEN** a member with one card removes it
- **THEN** the member SHALL have no default card and `auto-renew` SHALL no longer attempt to charge
