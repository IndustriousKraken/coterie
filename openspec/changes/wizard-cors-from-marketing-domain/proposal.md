# wizard-cors-from-marketing-domain

## Why

The wizard collects a **marketing domain** and configures a Caddy vhost for
it, but never populates `COTERIE__SERVER__CORS_ORIGINS`. The `cors-policy`
capability defaults to same-origin — `build_cors_layer`
(`src/api/mod.rs:85`) emits no `Access-Control-Allow-Origin` header unless
`cors_origins` is set. So after a standard install with a marketing domain,
the marketing site's browser calls to the public API (`/public/events`,
`/public/announcements`, `/public/feed/*`) are blocked by the same-origin
policy: the API returns `200`, but without the CORS header the browser
discards the response.

A separate marketing site reading events/announcements over the public API
is the headline reason to run a marketing domain alongside Coterie, yet the
wizard leaves it broken out of the box — every operator must discover the
problem in the browser console, hand-edit `.env`, and restart. The operator
already supplied the domain; wiring it into `cors_origins` at render time
closes the gap where the information is known.

- **Trigger:** install with a marketing domain, then load the marketing
  site; its `fetch` to `https://<portal>/public/events` is CORS-blocked.
- **Harm:** the public content feeds do not work until the operator
  manually finds and sets `cors_origins`.

This adds new wizard behavior (the rendered `.env` now carries a setting it
did not before), so it lands in the spec lane as an `ADDED` requirement on
`provisioning-wizard` rather than an issue.

## What Changes

- When the operator supplies a marketing domain, the wizard sets
  `COTERIE__SERVER__CORS_ORIGINS` in the rendered `.env` to the HTTPS
  origin(s) the Caddy marketing vhost serves for that domain (the apex and
  its `www.` variant), so the marketing site's browser requests to
  `/public/*` succeed.
- When no marketing domain is supplied, `cors_origins` stays unset
  (same-origin only) — unchanged.
- The runtime `cors-policy` behavior is unchanged; this only populates the
  setting the wizard already had the information to fill.

## Impact

- Spec: `provisioning-wizard` — `ADDED` requirement "Wizard configures CORS
  from the marketing domain".
- Code: `deploy/coterie-provision/src/env_template.rs` (derive
  `cors_origins` from the marketing domain; uncomment + fill the
  `COTERIE__SERVER__CORS_ORIGINS` line) and the `EnvConfig` call site in
  `deploy/coterie-provision/src/install.rs`.
- Tests: the `.env` golden snapshot / an `env_template` unit test asserts
  the CORS line is set for a marketing-domain run and absent otherwise.
- No database, wire-format, or API-shape changes. The exact origin list
  (apex + `www.`) is implementation guidance; the binding invariant is that
  the marketing site's browser origin is present in `cors_origins` after a
  marketing-domain install.
