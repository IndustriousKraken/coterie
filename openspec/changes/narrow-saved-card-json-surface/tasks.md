## 1. Verify the assumption

- [ ] 1.1 Run `grep -rn "/api/payments/cards" templates/ static/` and confirm the only matches are `setup-intent` and bare `/api/payments/cards` (POST). No `GET /api/payments/cards`, no `DELETE /api/payments/cards`, no `PUT /api/payments/cards`. If matches show up, abort and reconsider — the assumption underlying this change would be wrong.
- [ ] 1.2 Run `grep -rn "/api/payments/cards" tests/ src/` and identify any test or non-template caller of the three endpoints. Catalog them; they'll need migration in step 4.

## 2. Remove the route registrations

- [ ] 2.1 In `src/api/mod.rs`, delete the three route lines:
  ```rust
  .route("/cards", get(handlers::payments::list_saved_cards))
  .route("/cards/:card_id", delete(handlers::payments::delete_saved_card))
  .route("/cards/:card_id/default", put(handlers::payments::set_default_card))
  ```
- [ ] 2.2 Keep these route lines:
  ```rust
  .route("/cards", post(handlers::payments::save_card))
  .route("/cards/setup-intent", post(handlers::payments::create_setup_intent))
  ```
- [ ] 2.3 Verify the `auth` route_layer still applies to the remaining two routes.

## 3. Remove the handler functions

- [ ] 3.1 Delete `pub async fn list_saved_cards(...)` from `src/api/handlers/payments.rs`.
- [ ] 3.2 Delete `pub async fn delete_saved_card(...)` from `src/api/handlers/payments.rs`.
- [ ] 3.3 Delete `pub async fn set_default_card(...)` from `src/api/handlers/payments.rs`.
- [ ] 3.4 Check whether `CardResponse` and `impl From<SavedCard> for CardResponse` are still referenced (likely by the kept `save_card` handler). If yes, leave them. If no, delete them.
- [ ] 3.5 Sweep for unused imports in `src/api/handlers/payments.rs` after the deletions.

## 4. Migrate or delete affected tests

- [ ] 4.1 For each test catalogued in 1.2, decide:
  - If the test's intent is "can a member delete a card", migrate it to call `DELETE /portal/api/payments/cards/:id` and assert the HTML fragment response.
  - If the test's intent is "the JSON endpoint enforces auth", delete it (the JSON endpoint no longer exists; the same auth contract is verified for the kept JSON endpoints `setup-intent` and `POST /cards`).
- [ ] 4.2 If no tests target the deleted endpoints (likely), skip this section.

## 5. Verify the build

- [ ] 5.1 `cargo build --all-targets --features test-utils` — clean. Any errors are stragglers in tests or other consumers; fix.
- [ ] 5.2 `cargo test --features test-utils` — full suite passes.

## 6. Manual smoke test of the payment-methods page

- [ ] 6.1 Start the dev server, log in as a member with a Stripe-configured environment.
- [ ] 6.2 Visit `/portal/payments/methods`. The saved-card list renders (HTML endpoint).
- [ ] 6.3 Add a new card via the Stripe.js form. Confirm SetupIntent fetch (JSON `setup-intent`) and save fetch (JSON `POST /cards`) succeed; the new card appears in the list.
- [ ] 6.4 Click "Make default" on a non-default card. HTMX `PUT /portal/api/payments/cards/:id/default` updates the list fragment correctly.
- [ ] 6.5 Click "Remove" on a card. HTMX `DELETE /portal/api/payments/cards/:id` removes the card and updates the list.
- [ ] 6.6 Confirm `GET /api/payments/cards` returns 404 (e.g., via curl with a session cookie). The route is gone.

## 7. Spec sync

- [ ] 7.1 Confirm the change's delta specs (`openspec/changes/narrow-saved-card-json-surface/specs/member-saved-cards/spec.md` and `saved-card-management/spec.md`) match the implemented behavior.
- [ ] 7.2 At archive time (`opsx:archive`), the modified requirements merge into `openspec/specs/member-saved-cards/spec.md` and `openspec/specs/saved-card-management/spec.md`.
