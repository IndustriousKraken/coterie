## Why

Saved-card management has parallel JSON and HTML surfaces that overlap. The full picture today:

```
JSON  /api/payments/cards                  ← list_saved_cards
JSON  POST /api/payments/cards             ← save_card
JSON  POST /api/payments/cards/setup-intent ← create_setup_intent
JSON  DELETE /api/payments/cards/:id       ← delete_saved_card
JSON  PUT /api/payments/cards/:id/default  ← set_default_card

HTML  GET /portal/api/payments/cards               ← saved_cards_html_api
HTML  DELETE /portal/api/payments/cards/:id        ← delete_card_api
HTML  PUT /portal/api/payments/cards/:id/default   ← set_default_card_api
```

`templates/portal/payment_methods.html` and `_saved_card_list.html` make the actual frontend usage explicit:

- **Stripe.js** calls `POST /api/payments/cards/setup-intent` (to create the SetupIntent) and then `POST /api/payments/cards` (to persist the confirmed payment method). These are JSON-in / JSON-out by Stripe.js' design — load-bearing.
- **HTMX** calls `GET /portal/api/payments/cards` to render the card-list fragment, `DELETE /portal/api/payments/cards/:id` to remove a card, and `PUT /portal/api/payments/cards/:id/default` to mark a default.
- **Nothing** calls `GET /api/payments/cards`, `DELETE /api/payments/cards/:id`, or `PUT /api/payments/cards/:id/default`. They're vestigial.

CLAUDE.md states the rule explicitly: *"`/api/*` is intentionally narrow — not a CRUD API. […] saved-card management endpoints called directly by the portal frontend's Stripe.js integration."* The unused list/delete/set-default JSON endpoints widen the surface beyond what Stripe.js needs and create a "which one do I call?" fork for any future frontend code.

The fix is to delete the three unused JSON endpoints. Stripe.js's two endpoints stay (they have to). HTMX continues to use the HTML endpoints for the portal UI flows.

## What Changes

- **Remove three JSON endpoints** from `/api/payments/cards/*`:
  - `GET /api/payments/cards` (`list_saved_cards`)
  - `DELETE /api/payments/cards/:card_id` (`delete_saved_card`)
  - `PUT /api/payments/cards/:card_id/default` (`set_default_card`)
- **Remove the corresponding handler functions** in `src/api/handlers/payments.rs` (`list_saved_cards`, `delete_saved_card`, `set_default_card`).
- **Remove the route registrations** in `src/api/mod.rs`.
- **Keep two JSON endpoints** that Stripe.js needs:
  - `POST /api/payments/cards/setup-intent` (`create_setup_intent`)
  - `POST /api/payments/cards` (`save_card`)
- **Keep all three HTML endpoints** that HTMX uses unchanged:
  - `GET /portal/api/payments/cards`
  - `DELETE /portal/api/payments/cards/:card_id`
  - `PUT /portal/api/payments/cards/:card_id/default`
- **Templates and JavaScript** are already calling the HTML endpoints for delete/default/list and the JSON endpoints only for setup-intent/save. No frontend changes needed.
- **Spec deltas**:
  - `member-saved-cards`: replace the 5-endpoint JSON list with the actual 2-endpoint JSON surface plus the 3-endpoint HTML surface.
  - `saved-card-management`: unchanged on its top-level requirement (the SetupIntent → record-pm flow stays at `/api/payments/cards/*`); only minor language clarifying "the only endpoints under `/api/payments/cards/*` are setup-intent and POST" if needed.

## Capabilities

### New Capabilities

(None — the change narrows the existing capability surface; no new capability is introduced.)

### Modified Capabilities
- `member-saved-cards`: the surface description changes. JSON endpoints shrink to setup-intent + record-card (the two Stripe.js needs). The HTML endpoints under `/portal/api/payments/cards/*` for list/delete/set-default become spec'd as the canonical UI path. The "/api/* is the only legitimate JSON outside webhook" framing tightens accordingly.
- `saved-card-management`: light touch. The lifecycle requirement still names `POST /api/payments/cards/setup-intent` and `POST /api/payments/cards` — those stay. No behavioral change.

## Impact

- **Code**:
  - **Removed**: ~80 lines from `src/api/handlers/payments.rs` (three unused handler functions plus their request/response DTOs if they're handler-private).
  - **Removed**: 3 route registrations from `src/api/mod.rs`.
  - **Net**: ~80 lines removed; the JSON surface narrows to webhook + 2 saved-card endpoints, exactly matching the CLAUDE.md guidance.
- **Wire shape**: the three deleted endpoints return 404 after the change. Since nothing calls them today (verified: `grep -rn "/api/payments/cards" templates/ static/` shows only setup-intent + POST), no UI breaks. External consumers (none documented in OpenAPI; the `saved-card-management` flows are member-self only and were never part of the public API contract) similarly don't break.
- **Tests**: any test that calls the deleted endpoints needs to be removed or migrated to the HTML counterpart. Most existing tests target the SetupIntent + save flow (which stays) or the HTMX HTML flow (which stays).
- **Risk**: low. The frontend usage is grep-verifiable: Stripe.js → JSON setup-intent + save; everything else → HTML. A consumer outside this codebase (ops scripts, etc.) is unlikely given the auth requirement (`require_auth` member-self gate).
- **Trade-off**: a future contributor who wants a JSON list of cards (e.g., for a mobile client) would need to re-add the endpoint. That's acceptable — adding a route is cheap; carrying unused routes indefinitely is the cost we're paying down.
- **Documentation**: `ARCHITECTURE.md` and `CLAUDE.md` already describe `/api/*` as narrow. The narrowing this change enacts brings the code in line with the documentation, not the other way around.
