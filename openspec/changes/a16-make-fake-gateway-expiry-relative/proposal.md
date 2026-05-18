## Why

`src/payments/fake_gateway.rs:273` returns a hardcoded `exp_year: 2030` as the default `PaymentMethodDetails` from `retrieve_payment_method` when no canned response is queued. This is the "default valid card" test fixture — tests that don't explicitly queue a response for `retrieve_payment_method` get back a card "valid through December 2030."

When wall-clock time reaches 2030, that card becomes effectively expired. Tests that implicitly rely on the default response being "a valid card" will start producing unexpected behavior: card-validity checks return false, auto-renew flows skip the card with "expired card" reasons, etc. Roughly four years of runway, but the pattern is the same trap as the test-anchor drift `a14` / `a15` fixed — a hardcoded future date that's a time-bomb.

The fix is one line: compute the year at runtime so the "default valid card" is always 5 years out from when the test runs.

## What Changes

- **In `src/payments/fake_gateway.rs::retrieve_payment_method`**, replace `exp_year: 2030` with `exp_year: chrono::Utc::now().year() + 5`. The `+5` matches the spirit of the original "comfortably future" intent. `exp_month: 12` stays — December is the last month of the year, so combined with `year + 5` it means the fake card is valid through December of (current year + 5).
- **Add the import**: `use chrono::{Datelike, Utc};` at the top of the file (or extend an existing import). `Datelike` is what provides the `.year()` method on `DateTime<Utc>`.
- **Out of scope**: every other hardcoded date in `src/`. The earlier sweep confirmed they're either (a) drift candidates already queued (`a15` for `event_admin_service.rs`), or (b) deliberate pure-function inputs that SHOULD stay hardcoded.

## Capabilities

### New Capabilities

(None.)

### Modified Capabilities
- `saved-card-management`: adds a small requirement that test fixtures representing "valid" saved cards SHALL use runtime-relative expiry dates so they don't silently age into "expired" as wall-clock time advances. Codifies the principle behind this one-line fix so a future contributor doesn't reintroduce the pattern in another fixture.

## Impact

- **Code**: one line changed in `src/payments/fake_gateway.rs`, plus a possible import adjustment. Net: ~1–2 lines.
- **Wire shape**: no production runtime change. The fake gateway is `#[cfg(feature = "test-utils")]`-gated; nothing in production code paths.
- **Tests**: existing tests continue to pass — the fake's behavior is unchanged in 2026 (it returns `exp_year: 2031` instead of `exp_year: 2030`, both "comfortably future"). Tests that assert specifically on the exact `exp_year` value (none expected, but worth checking) would need a small adjustment.
- **Risk**: trivial.
- **Dependency**: none.
