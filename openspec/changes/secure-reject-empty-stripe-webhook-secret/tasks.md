## 1. Treat a blank Stripe webhook secret (and API key) as unconfigured

- [ ] 1.1 In `src/main.rs`, immediately before the `match (settings.stripe.secret_key.clone(), settings.stripe.webhook_secret.clone())` at line ~212, bind normalized locals, e.g.:
  ```rust
  let secret_key = settings.stripe.secret_key.clone().filter(|s| !s.trim().is_empty());
  let webhook_secret = settings.stripe.webhook_secret.clone().filter(|s| !s.trim().is_empty());
  ```
  and match on `(secret_key.clone(), webhook_secret.clone())` instead of the raw `settings.stripe.*` values, so `Some("")` / `Some("   ")` become `None`.
- [ ] 1.2 In the webhook-dispatcher wiring at `src/main.rs:434-449`, use the same normalized `webhook_secret` local (not `settings.stripe.webhook_secret.clone()`) so a blank secret yields `webhook_dispatcher = None` and the endpoint falls through to the existing `503` path in `src/api/handlers/payments.rs:47-50`.
- [ ] 1.3 Keep the existing `tracing::warn!("Stripe enabled but missing configuration")` arm so a deployment with `stripe.enabled = true` but a blank key/secret logs the misconfiguration at boot.

## 2. Spec update

- [ ] 2.1 Add the `ADDED Requirements` block from `specs/stripe-webhook/spec.md` in this change to `openspec/specs/stripe-webhook/spec.md`: a blank/whitespace webhook secret SHALL be treated as unconfigured and SHALL NOT enable webhook processing.

## 3. Tests

- [ ] 3.1 Factor the blank-to-`None` normalization into a small pure helper (e.g. `fn nonblank(s: Option<String>) -> Option<String>`) in `src/config/mod.rs` or `src/main.rs`, and add a unit test asserting `nonblank(Some("".into())) == None`, `nonblank(Some("  ".into())) == None`, `nonblank(Some("whsec_x".into())) == Some("whsec_x".into())`, and `nonblank(None) == None`.
- [ ] 3.2 Add a test that constructs a `StripeConfig { enabled: true, secret_key: Some("sk_test_x".into()), webhook_secret: Some("".into()), .. }`, runs it through the normalization used by the wiring, and asserts the webhook-secret side resolves to `None` (i.e. no dispatcher would be built). Keep the assertion at the config/normalization layer so it needs no live server or network.
