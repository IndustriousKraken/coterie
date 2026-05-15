## Context

The saved-card surface accumulated five JSON endpoints under `/api/payments/cards/*` and three near-parallel HTML endpoints under `/portal/api/payments/cards/*`. The history is plausible: the JSON surface predates the HTMX-portal pattern, and as the portal frontend stabilized on HTMX for non-Stripe.js flows, the HTML endpoints were added without retiring the now-redundant JSON ones.

Verification of actual usage (`grep -rn "/api/payments/cards" templates/ static/`) shows:

- `templates/portal/payment_methods.html`: Stripe.js fetches `POST /api/payments/cards/setup-intent` and `POST /api/payments/cards`. Nothing else under `/api/*`.
- `templates/portal/_saved_card_list.html`: HTMX `hx-delete` and `hx-put` against `/portal/api/payments/cards/*`.
- `templates/portal/payment_methods.html`: HTMX `hx-get="/portal/api/payments/cards"` reloads the list fragment after card actions.

So three of the five JSON endpoints (`GET`, `DELETE`, `PUT default`) have no frontend caller. They exist only because they were once the planned interface for the portal — superseded by the HTML versions.

The other two JSON endpoints (`setup-intent`, `POST /cards`) are load-bearing and *cannot* be HTML — Stripe.js calls them directly with `fetch()` and expects JSON. Those stay.

## Goals / Non-Goals

**Goals:**
- The `/api/*` JSON surface narrows to exactly what's required by Stripe.js (setup-intent + record-pm) plus the Stripe webhook. This matches the CLAUDE.md framing word-for-word.
- Frontend code has one canonical path for each saved-card action: Stripe.js → JSON endpoints; UI interactions (list, delete, set default) → HTML endpoints.
- Dead handler code (~80 lines) is removed, not left as "in case someone wants this later."

**Non-Goals:**
- Changing the HTML endpoints under `/portal/api/payments/cards/*`. They stay as-is.
- Changing the SetupIntent flow or any Stripe.js interaction.
- Re-implementing card management in HTMX-only style (the JSON setup-intent + record-pm calls *must* be JSON; Stripe.js requires it).
- Touching any other `/api/*` endpoint (the Stripe webhook, payment routes that aren't card-related).
- Adding new capability (this is purely a removal).
- Renaming the HTML endpoints (e.g., dropping the `/api/` infix in `/portal/api/payments/cards`). That's a separate cosmetic concern.

## Decisions

### D1. Delete the handler functions, not just the routes

`list_saved_cards`, `delete_saved_card`, and `set_default_card` in `src/api/handlers/payments.rs` are removed entirely — handler bodies plus request/response DTOs that exist solely to serve them. Considered: leave the handlers but unwire the routes. Rejected — a `#[allow(dead_code)]` zombie handler is worse than no handler. CLAUDE.md is clear on this: don't carry unused code "in case."

### D2. Keep the `SavedCard → CardResponse` `From` impl

`src/api/handlers/payments.rs:119` defines `impl From<SavedCard> for CardResponse`. `CardResponse` is also referenced from the HTML side indirectly via the same `SavedCard` projection. Whether `CardResponse` itself becomes dead code depends on whether `save_card` (which stays) returns it. Verify in implementation; if `save_card` returns `CardResponse`, it stays; if not, it's deleted alongside the deleted handlers.

### D3. The HTMX flow remains the primary path; spec calls it out explicitly

After this change, `member-saved-cards` says: "Stripe.js uses two JSON endpoints; everything else (listing, removal, default-flag) is HTML under `/portal/api/payments/cards/*`." The HTML endpoints become first-class spec citizens, not after-thoughts.

### D4. CSRF and auth contracts are unchanged

The remaining JSON endpoints already require valid CSRF tokens (top-level layer) and `require_auth` (member-self) — no change. The HTML endpoints already require valid CSRF + `require_restorable` (so Expired members can manage cards as part of the dues-restoration flow). The deletion doesn't alter any gate.

### D5. OpenAPI documentation is unaffected

Saved-card endpoints aren't in the OpenAPI spec (only the public marketing-site surface is). Nothing in `src/api/docs.rs` references them. No documentation update needed beyond the spec deltas.

### D6. No deprecation period

Considered: respond with `Gone (410)` for some grace period before deletion to catch unexpected callers. Rejected:
- The endpoints require auth, so any caller would be a logged-in member — discoverable via session-id in logs if they show up.
- The codebase is single-tenant; no third-party integration relies on these.
- "Gone" responses would still need handler code to emit them, defeating the cleanup.
- A simple `git revert` is the rollback if a regression surfaces.

### D7. Tests migrate or delete in lockstep

Any test asserting the deleted JSON endpoints' behavior either:
- Migrates to call the corresponding HTML endpoint (if the test's actual concern is "can a member delete a card"), or
- Is deleted (if the test was specifically about the JSON contract and the HTML version has its own test).

Tests that exercise the SetupIntent / record-pm flow stay; those endpoints stay.

### D8. Specs update both impacted capabilities, but the bulk of change is in `member-saved-cards`

`saved-card-management` keeps its lifecycle requirement (SetupIntent → confirm → record-pm). The naming of `POST /api/payments/cards` and `POST /api/payments/cards/setup-intent` doesn't change. The only language drift is removing any phrase that implies the JSON surface includes list/delete/set-default — easy.

`member-saved-cards` is the bigger update: its existing requirement enumerates all five JSON endpoints. This change replaces that with two JSON endpoints + three HTML endpoints under `/portal/api/payments/cards/*`. The "list of permitted JSON endpoints under `/api/*`" requirement updates accordingly.

## Risks / Trade-offs

- **Risk**: an undocumented external script or admin tool calls one of the deleted endpoints. → **Mitigation**: search for the URLs in the repo (templates, static, docs) — already done; nothing references them. Production traffic logs would surface real callers; if any appear post-deploy, restore the endpoint with a one-line route + a wrapper that calls the corresponding HTML handler's logic (or just `git revert`).
- **Risk**: a future Stripe.js update needs the JSON list endpoint. → **Mitigation**: today Stripe.js doesn't list cards — it confirms a SetupIntent and produces a payment-method id. If Stripe's flow changes, re-add the endpoint. Cost of re-adding < cost of indefinite carrying.
- **Trade-off**: `/api/payments/cards/setup-intent` and `POST /api/payments/cards` continue to live "alone" under `/api/payments/cards/*` without their list/delete/set-default siblings. That asymmetry is the point — it makes the JSON surface's purpose ("Stripe.js entry points") legible from the route table itself.

## Migration Plan

Single-PR removal; no DB changes, no flags.

1. Remove route registrations in `src/api/mod.rs` for `GET /cards`, `DELETE /cards/:id`, `PUT /cards/:id/default`.
2. Remove handler functions `list_saved_cards`, `delete_saved_card`, `set_default_card` from `src/api/handlers/payments.rs`. Remove any DTO that's now unreferenced.
3. Migrate or delete tests that exercised the deleted endpoints.
4. `cargo build --all-targets --features test-utils`.
5. `cargo test --features test-utils`.
6. Spot-check the payment-methods page in the portal: list renders (HTML endpoint), delete works (HTML endpoint), set-default works (HTML endpoint), add-card flow works end-to-end (JSON setup-intent + JSON save).
7. Deploy normally. `git revert` is the rollback.
