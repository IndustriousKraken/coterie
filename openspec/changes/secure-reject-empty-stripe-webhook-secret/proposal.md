## Why

The Stripe wiring in `src/main.rs` is documented to enable Stripe "only when both an API key AND a webhook secret are configured; missing either disables Stripe entirely" (`src/main.rs:209-211`). The actual check does NOT enforce that invariant for a *blank* value:

```rust
// src/main.rs:212-217
let stripe_client = if settings.stripe.enabled {
    match (
        settings.stripe.secret_key.clone(),
        settings.stripe.webhook_secret.clone(),
    ) {
        (Some(api_key), Some(_)) => { /* Stripe enabled */ }
```

`webhook_secret` is an `Option<String>` (`src/config/mod.rs:169`) and the shipped `config.toml:32` sets `webhook_secret = ""`. An empty string deserializes to `Some("")`, which matches `Some(_)`. The dispatcher is then built with that empty secret (`src/main.rs:434-449` — `webhook_secret.clone().map(|webhook_secret| WebhookDispatcher::new(.., webhook_secret, ..))`), and verification runs against it (`src/payments/webhook_dispatcher/mod.rs:78` — `Webhook::construct_event(payload, sig, &self.webhook_secret)`).

HMAC-SHA256 accepts a zero-length key, so `construct_event` with an empty secret still "verifies" — but the signature is `HMAC-SHA256("", "{t}.{payload}")`, which **any unauthenticated caller can compute themselves**. The endpoint also only requires the timestamp to be within Stripe's ±300s tolerance, which an attacker trivially satisfies with a current timestamp.

Attacker / input: an unauthenticated POST to `/api/payments/webhook/stripe` with a self-computed `Stripe-Signature` header, when the operator has set `stripe.enabled = true` and a real `secret_key` but left `webhook_secret` blank (the value the default `config.toml` ships, and the value produced by an unset `COTERIE__STRIPE__WEBHOOK_SECRET=` env override).

Harm: forged webhook events are processed as genuine. A forged `checkout.session.completed` / `payment_intent.succeeded` can flip a Pending `payments` row to Completed and **extend a member's dues without any real payment** (`src/payments/webhook_dispatcher/checkout.rs`, `payment_intent.rs`); a forged `charge.refunded` can mark payments Refunded; a forged `customer.subscription.deleted` can flip a member's billing mode. This is an authentication bypass on a money-moving endpoint.

The correct fail-safe already exists: when the dispatcher is `None`, the handler returns `503` without processing (`src/api/handlers/payments.rs:47-50`). The fix is simply to make a blank secret resolve to `None` (i.e. "not configured") instead of `Some("")`.

## What Changes

In `src/main.rs`, before the `match`, normalize blank Stripe credentials to `None` so the existing "missing either disables Stripe" logic treats an empty string the same as an absent value:

- Treat `webhook_secret` whose trimmed value is empty as not configured (→ `None`), so the webhook dispatcher is built as `None` and the endpoint returns `503` instead of accepting forged events.
- Apply the same normalization to `secret_key` so a blank API key cannot enable a half-configured Stripe.
- When `stripe.enabled = true` but either normalized value is `None`, keep the existing `tracing::warn!("Stripe enabled but missing configuration")` so the misconfiguration is visible in logs at boot.

Add an `ADDED` requirement to the `stripe-webhook` spec stating that a blank/whitespace webhook secret SHALL be treated as unconfigured and SHALL NOT enable webhook processing.

## Impact

- `src/main.rs` — normalize `settings.stripe.secret_key` and `settings.stripe.webhook_secret` with `.filter(|s| !s.trim().is_empty())` before the wiring match (covers both the file-config default `webhook_secret = ""` and the env-var `COTERIE__STRIPE__WEBHOOK_SECRET=` path).
- `openspec/specs/stripe-webhook/spec.md` — new requirement: blank webhook secret disables the endpoint.
- Tests: a unit test asserting the normalization helper maps `Some("")` / `Some("   ")` → `None`, plus an integration-style assertion that a `WebhookDispatcher` is not constructed when the secret is blank.

Operator follow-up (NOT an implementer task): any deployment currently running with `stripe.enabled = true` and a blank `webhook_secret` has been accepting forgeable webhooks; after this fix the endpoint returns `503` until a real secret is set. Such deployments SHOULD set a genuine `whsec_…` value and audit recent Pending→Completed payment transitions for ones not backed by a real Stripe charge.
