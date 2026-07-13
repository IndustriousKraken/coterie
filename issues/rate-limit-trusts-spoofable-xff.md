# Rate limiter trusts a spoofable X-Forwarded-For entry

## Summary

`client_ip` (`src/api/state.rs:43`) derives the per-request client IP — the key
for BOTH `money_limiter` and `login_limiter` — from the **left-most** entry of
the `X-Forwarded-For` header when `trust_forwarded_for` is enabled. The
left-most entry is client-supplied: a standard reverse proxy (Caddy, in the
reference deployment) **appends** the connecting peer to `X-Forwarded-For`, so a
request that arrives already carrying `X-Forwarded-For: 1.2.3.4` reaches Coterie
as `X-Forwarded-For: 1.2.3.4, <real-client>`. Taking `.split(',').next()` yields
`1.2.3.4` — a value the attacker fully controls and can rotate per request,
defeating both rate limiters and spoofing the `remoteip` passed to the
bot-challenge provider.

This only bites when `trust_forwarded_for` is enabled — which the provisioning
wizard turns on for any `https` `base_url`, i.e. the normal production posture.
When it is disabled the code already ignores the header and collapses to a
single bucket (safe).

## Source location

`src/api/state.rs:43-51`:

```rust
pub fn client_ip(headers: &HeaderMap, trust_forwarded: bool) -> IpAddr {
    if trust_forwarded {
        // Try X-Forwarded-For (first IP in the chain is the client)
        if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
            if let Some(first) = xff.split(',').next() {          // <-- left-most = attacker-controlled
                if let Ok(ip) = first.trim().parse::<IpAddr>() {
                    return ip;
                }
            }
        }
        ...
```

The comment "first IP in the chain is the client" is the bug: the left-most
entry is the *claimed* client, not the *verified* one. With one trusted proxy
in front, the verified client is the **right-most** entry (the peer that proxy
actually received the connection from).

## Why this is harmful

`money_limiter` (10/min per IP) is the primary defense on `/public/donate` and
payment-mode `/public/signup` — the card-testing / carding surface — and
`login_limiter` (5 per 15 min) is the brute-force defense on `/auth/login`,
`/login`, the TOTP endpoints, and `/forgot-password`. An attacker who sets a
fresh random `X-Forwarded-For` per request gets a fresh rate-limit bucket every
time, nullifying both limiters. It also lets them feed an arbitrary `remoteip`
to the bot-challenge provider.

- **Trigger:** any request to a rate-limited endpoint with an attacker-supplied
  `X-Forwarded-For` header, in a deployment with `trust_forwarded_for` enabled.
- **Harm:** rate limiting and login brute-force protection are bypassed;
  bot-challenge `remoteip` is spoofed.

## Acceptance criteria (against existing specification)

This corrects code to fulfill an existing requirement; it adds no new contract.

- **`rate-limiting` → Requirement "Money-moving endpoints are rate-limited per
  IP"** and **"Credential flows are rate-limited per IP":** these require a
  *per-IP* limit. The limiter must key on the real client IP, which a
  client-controlled header value is not. After the fix, an attacker cannot
  obtain a fresh bucket by varying `X-Forwarded-For`.

Concretely:

1. When `trust_forwarded_for` is enabled, `client_ip` MUST resolve the IP from
   the **right-most** `X-Forwarded-For` entry (the hop appended by the single
   trusted proxy), not the left-most, so a client-prepended value cannot become
   the rate-limit key. (If a configurable trusted-proxy hop count is preferred,
   default it to 1 to match the reference single-Caddy deployment.)
2. Behavior when `trust_forwarded_for` is disabled is unchanged (header ignored;
   single-bucket fallback).
3. `X-Real-Ip` handling is unchanged (a single proxy sets it to the peer).

## Tasks

- [ ] 1.1 In `src/api/state.rs::client_ip`, change the `X-Forwarded-For` parse
  to take the last (right-most) comma-separated entry instead of the first;
  keep the `trim().parse::<IpAddr>()` guard and the `X-Real-Ip` / loopback
  fallbacks intact.
- [ ] 2.1 Add a unit test in `src/api/state.rs` (or `tests/`) asserting that,
  with `trust_forwarded_for = true`, `client_ip` for
  `X-Forwarded-For: 1.2.3.4, 9.9.9.9` returns `9.9.9.9` (the trusted-proxy hop),
  NOT `1.2.3.4`; and that a lone `X-Forwarded-For: 9.9.9.9` still returns
  `9.9.9.9`.
- [ ] 2.2 Add a test asserting that with `trust_forwarded_for = false` the
  header is ignored (loopback fallback), guarding the safe-by-default path.
- [ ] 3.1 Run `cargo test` and confirm the rate-limiting-adjacent suites pass.
