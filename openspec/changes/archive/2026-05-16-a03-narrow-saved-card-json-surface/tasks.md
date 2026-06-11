## 1. Verify the assumption

- [x] 1.1 Run `grep -rn "/api/payments/cards" templates/ static/` and confirm the only matches are `setup-intent` and bare `/api/payments/cards` (POST). No `GET /api/payments/cards`, no `DELETE /api/payments/cards`, no `PUT /api/payments/cards`. If matches show up, abort and reconsider — the assumption underlying this change would be wrong.
- [x] 1.2 Run `grep -rn "/api/payments/cards" tests/ src/` and identify any test or non-template caller of the three endpoints. Catalog them; they'll need migration in step 4.

## 2. Remove the route registrations

- [x] 2.1 In `src/api/mod.rs`, delete the three route lines:
  ```rust
  .route("/cards", get(handlers::payments::list_saved_cards))
  .route("/cards/:card_id", delete(handlers::payments::delete_saved_card))
  .route("/cards/:card_id/default", put(handlers::payments::set_default_card))
  ```
- [x] 2.2 Keep these route lines:
  ```rust
  .route("/cards", post(handlers::payments::save_card))
  .route("/cards/setup-intent", post(handlers::payments::create_setup_intent))
  ```
- [x] 2.3 Verify the `auth` route_layer still applies to the remaining two routes.

## 3. Remove the handler functions

- [x] 3.1 Delete `pub async fn list_saved_cards(...)` from `src/api/handlers/payments.rs`.
- [x] 3.2 Delete `pub async fn delete_saved_card(...)` from `src/api/handlers/payments.rs`.
- [x] 3.3 Delete `pub async fn set_default_card(...)` from `src/api/handlers/payments.rs`.
- [x] 3.4 Check whether `CardResponse` and `impl From<SavedCard> for CardResponse` are still referenced (likely by the kept `save_card` handler). If yes, leave them. If no, delete them.
- [x] 3.5 Sweep for unused imports in `src/api/handlers/payments.rs` after the deletions.

## 4. Migrate or delete affected tests

- [x] 4.1 For each test catalogued in 1.2, decide:
  - If the test's intent is "can a member delete a card", migrate it to call `DELETE /portal/api/payments/cards/:id` and assert the HTML fragment response.
  - If the test's intent is "the JSON endpoint enforces auth", delete it (the JSON endpoint no longer exists; the same auth contract is verified for the kept JSON endpoints `setup-intent` and `POST /cards`).
- [x] 4.2 If no tests target the deleted endpoints (likely), skip this section.

## 5. Verify the build

- [x] 5.1 `cargo build --all-targets --features test-utils` — clean. Any errors are stragglers in tests or other consumers; fix.
- [x] 5.2 `cargo test --features test-utils` — full suite passes.

## 6. Integration tests for the surviving card flows

Goal: prove (without a real browser or real Stripe) that the kept JSON endpoints and the HTML endpoints both work end-to-end, and that the deleted JSON endpoints actually return 404. Use the existing test harness — `FakeStripeGateway` (gated by `--features test-utils`) and an in-memory SQLite pool. Reference patterns: `tests/stripe_gateway_test.rs`, `tests/stripe_webhook_test.rs`.

- [x] 6.1 Create `tests/saved_card_routes_test.rs` (new file) that boots a `Router` via `coterie::api::create_app(app_state)` + `coterie::web::create_web_routes(app_state)`, wired to an in-memory SQLite pool with migrations applied and a `FakeStripeGateway`-backed `StripeClient`. Use `tower::ServiceExt::oneshot` to drive requests through the router without binding a TCP socket.
- [x] 6.2 Test `deleted_json_endpoints_return_404`: send `GET /api/payments/cards`, `DELETE /api/payments/cards/<some-uuid>`, and `PUT /api/payments/cards/<some-uuid>/default` (each with a valid session cookie + CSRF token) and assert each response is `404 NOT_FOUND`. This is the regression net for "did the route actually get unregistered."
- [x] 6.3 Test `setup_intent_flow_still_works`: log in as a test member (helper: write a session row directly), POST to `/api/payments/cards/setup-intent` with a valid CSRF token, assert `200 OK` and that the response JSON contains a `client_secret`. Confirm the `FakeStripeGateway` recorded a `CreateSetupIntent` call.
- [x] 6.4 Test `save_card_flow_still_works`: with the fake gateway primed to accept an attached payment method, POST to `/api/payments/cards` with a `pm_*` id, assert `201 CREATED` (or whatever the handler returns today — check before writing the assertion) and that a `saved_cards` row exists for the test member.
- [x] 6.5 Test `html_list_endpoint_returns_fragment`: GET `/portal/api/payments/cards`, assert `200 OK`, content-type `text/html`, and that the response body contains a known marker from `_saved_card_list.html` (e.g., the `data-card-id` attribute or a class name).
- [x] 6.6 Test `html_delete_endpoint_works`: seed a `saved_cards` row, DELETE `/portal/api/payments/cards/<card_id>` with a valid CSRF token, assert `200 OK` (HTML fragment response) and that the row is gone from the DB.
- [x] 6.7 Test `html_set_default_endpoint_works`: seed two `saved_cards` rows for one member (only the first marked default), PUT `/portal/api/payments/cards/<other_card_id>/default`, assert `200 OK` and that the DB now shows the other card as default and the original as not-default.
- [x] 6.8 Run `cargo test --features test-utils --test saved_card_routes_test` and confirm all subtests pass.

## 7. Spec sync

- [x] 7.1 Confirm the change's delta specs (`openspec/changes/a03-narrow-saved-card-json-surface/specs/member-saved-cards/spec.md` and `saved-card-management/spec.md`) match the implemented behavior.

## Implementation notes

- Task 6.2 deviation from the task wording: `GET /api/payments/cards` returns **405 Method Not Allowed**, not 404, because the `POST /api/payments/cards` route is still registered at that path — axum responds 405 when a path is occupied with different methods. `DELETE /api/payments/cards/:id` and `PUT /api/payments/cards/:id/default` return 404 as expected (no route matches the path). The test (renamed `deleted_json_endpoints_have_no_handler`) asserts the correct per-case status. The `member-saved-cards` spec was updated to spell this out — the semantic is still "no handler reachable for these method+path combos."
- Task 3.4: `CardResponse` was named `SavedCardResponse` in this codebase. It's still used by the kept `save_card` handler, so it stays (along with its `impl From<SavedCard> for SavedCardResponse`).
- Existing `tests/csrf_coverage_test.rs::api_payments_post_without_csrf_returns_403` targets the kept `POST /api/payments/cards` — unaffected by the deletion.
- Pre-existing unrelated test failure: `tests/recurring_event_test.rs::weekly_creates_about_52_occurrences` (date-arithmetic flakiness; anchors at 2026-05-05 and expects 50–53 occurrences over a 52-week horizon, gets 54 on the current date).
