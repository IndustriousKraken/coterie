# stripe-webhook Specification Delta

## ADDED Requirements

### Requirement: Blank webhook secret is treated as unconfigured

A blank or whitespace-only Stripe webhook secret SHALL be treated as "not configured" and SHALL NOT enable webhook processing. The application startup wiring SHALL normalize `stripe.webhook_secret` (and `stripe.secret_key`) so that `Some("")` or `Some("   ")` resolves to `None`, the same as an absent value. When `stripe.enabled = true` but the normalized webhook secret is `None`, the `WebhookDispatcher` SHALL NOT be constructed and `POST /api/payments/webhook/stripe` SHALL return `503 Service Unavailable` without parsing or acting on the request body.

This closes a forgery path: an empty secret would otherwise be used as an HMAC-SHA256 key (which accepts a zero-length key), letting any unauthenticated caller compute a "valid" `Stripe-Signature` for an arbitrary payload and have forged events processed as genuine.

#### Scenario: Empty-string webhook secret does not enable the endpoint

- **WHEN** the application boots with `stripe.enabled = true`, a non-blank `secret_key`, and `webhook_secret = ""` (the value shipped in `config.toml`, or an unset `COTERIE__STRIPE__WEBHOOK_SECRET=` env override)
- **THEN** the webhook dispatcher SHALL NOT be constructed AND a POST to `/api/payments/webhook/stripe` SHALL return `503` without processing the payload

#### Scenario: Whitespace-only webhook secret is rejected

- **WHEN** `webhook_secret` is set to a whitespace-only string such as `"   "`
- **THEN** startup SHALL treat it as unconfigured (normalized to `None`) exactly as an empty string

#### Scenario: Genuine webhook secret still enables the endpoint

- **WHEN** the application boots with `stripe.enabled = true`, a non-blank `secret_key`, and a non-blank `webhook_secret` (e.g. `whsec_…`)
- **THEN** the webhook dispatcher SHALL be constructed and signature verification SHALL run against the configured secret as before
