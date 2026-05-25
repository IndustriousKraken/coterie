# member-saved-cards Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Card removal clears default if needed

Removing the member's only card, or removing the card currently marked default, SHALL update the member's default-card state to a sensible value (no default if no cards remain; first remaining card otherwise) atomically. This SHALL apply to both removal paths today — the (deleted) JSON `DELETE /api/payments/cards/:id` and the (kept) HTML `DELETE /portal/api/payments/cards/:id`.

After this change, only the HTML path remains.

#### Scenario: Removing the default card promotes another

- **WHEN** a member with two cards removes the one marked default via `DELETE /portal/api/payments/cards/:id`
- **THEN** the remaining card SHALL be marked default in the same transaction

### Requirement: Saved-card management uses /api/* JSON only for Stripe.js, HTMX HTML for everything else

Saved-card management for the member's own cards SHALL split between two surfaces:

**JSON endpoints under `/api/payments/cards/*`** (kept narrow because Stripe.js calls these directly via `fetch()` and requires JSON in / JSON out):

- `POST /api/payments/cards/setup-intent` — create a Stripe SetupIntent.
- `POST /api/payments/cards` — record a saved card after Stripe.js confirms the SetupIntent.

These two endpoints SHALL be the *only* saved-card endpoints under `/api/*`. They SHALL be gated by `require_auth` (member-self only). Together with the Stripe webhook, they SHALL be the only legitimate JSON endpoints under `/api/*`.

**HTML endpoints under `/portal/api/payments/cards/*`** (used by HTMX from the portal payment-methods page):

- `GET /portal/api/payments/cards` — render the saved-card list as an HTML fragment for HTMX swap.
- `DELETE /portal/api/payments/cards/:card_id` — remove a saved card; returns the updated list fragment.
- `PUT /portal/api/payments/cards/:card_id/default` — mark a saved card as default; returns the updated list fragment.

These three endpoints SHALL be gated by `require_restorable` (Active, Honorary, Expired members; Expired access is part of the dues-restoration flow). CSRF SHALL be enforced via the top-level layer; HTMX stamps the token on every `hx-*` request.

A `GET /api/payments/cards`, `DELETE /api/payments/cards/:id`, or `PUT /api/payments/cards/:id/default` handler SHALL NOT exist. `DELETE /api/payments/cards/:id` and `PUT /api/payments/cards/:id/default` SHALL return 404 (no route matches the path). `GET /api/payments/cards` SHALL return 405 Method Not Allowed (the path is occupied by the `POST` save-card handler; no GET handler is registered for it). In both cases the semantic is the same: the deleted endpoints reach no handler.

#### Scenario: Anonymous request to setup-intent returns 401

- **WHEN** an anonymous request hits `POST /api/payments/cards/setup-intent`
- **THEN** the response SHALL be 401 Unauthorized

#### Scenario: Anonymous request to record-card returns 401

- **WHEN** an anonymous request hits `POST /api/payments/cards`
- **THEN** the response SHALL be 401 Unauthorized

#### Scenario: Stripe.js fetches use the X-CSRF-Token header

- **WHEN** Stripe.js calls `fetch('/api/payments/cards/setup-intent')` or `fetch('/api/payments/cards')` from a portal page
- **THEN** the request SHALL include `X-CSRF-Token` from the rendered `<meta name="csrf-token">` tag, validated by the top-level CSRF layer

#### Scenario: HTMX list/delete/set-default return HTML fragments, not JSON

- **WHEN** the portal payment-methods page invokes `hx-get="/portal/api/payments/cards"`, `hx-delete="/portal/api/payments/cards/:id"`, or `hx-put="/portal/api/payments/cards/:id/default"`
- **THEN** the response SHALL be an HTML fragment (the saved-card list rendered to HTML), not JSON

#### Scenario: GET /api/payments/cards is gone

- **WHEN** any caller (script, ops tool, browser request) hits `GET /api/payments/cards`
- **THEN** the response SHALL be 405 Method Not Allowed (the path is registered for `POST` only; no GET handler exists)

#### Scenario: DELETE /api/payments/cards/:id is gone

- **WHEN** any caller hits `DELETE /api/payments/cards/<some-uuid>`
- **THEN** the response SHALL be 404 (no route matches that path pattern)

#### Scenario: PUT /api/payments/cards/:id/default is gone

- **WHEN** any caller hits `PUT /api/payments/cards/<some-uuid>/default`
- **THEN** the response SHALL be 404 (no route matches that path pattern)

#### Scenario: Member sees only their own cards via the HTML endpoint

- **WHEN** a member's portal page fetches `/portal/api/payments/cards`
- **THEN** the rendered HTML fragment SHALL include only cards owned by that member

