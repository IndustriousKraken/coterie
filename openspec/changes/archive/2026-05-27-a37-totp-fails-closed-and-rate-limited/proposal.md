## Why

The `secure-enforce-2fa-on-json-login` change closed the primary 2FA-bypass on `POST /auth/login`. Reviewing that fix surfaced two pre-existing weaknesses in the wider TOTP login surface — present in BOTH the web `/login` + `/login/totp` flow and the new JSON `/auth/login` + `/auth/login/totp` flow:

1. **`totp_service.is_enabled(member.id).await.unwrap_or(false)` fails open.** Both `src/api/handlers/auth.rs::login` and `src/web/templates/auth.rs::login_handler` use this pattern after the password check. If the underlying query errors (transient DB issue, pool exhaustion, etc.), the handler treats the member as not-enrolled, skips the 2FA branch, and issues a session. A network blip during the enrollment check lets a password-only authenticated request through with no second factor — the very thing 2FA exists to prevent.

2. **No rate limiting on the second-factor endpoints.** `POST /login/totp` and `POST /auth/login/totp` accept TOTP/recovery-code submissions but do NOT consult `login_limiter` (the per-IP `RateLimiter` newtype that `/auth/login` and `/login` both use). With a 5-minute pending TTL and 6-digit TOTP codes (1M-code space), an attacker holding a stolen password gets up to 5 minutes to try codes against the second-factor endpoint with no rate limit. Practical attack rate (limited by network + handler latency) is 100–1000 codes/sec, putting the success probability at 3–30% per pending session.

Together, these two gaps mean the 2FA enforcement is weaker than the spec promises: an attacker either flips a coin on the enrollment check failing (path 1) or brute-forces the second factor (path 2). Closing both is small surgery.

## What Changes

- **Fix #1 — fail-closed `is_enabled` check.** In both `src/api/handlers/auth.rs::login` and `src/web/templates/auth.rs::login_handler`, replace `.unwrap_or(false)` with `?` propagation (with context). A failure to query TOTP enrollment SHALL surface as a 500 response, not as a silent skip of the 2FA branch. The branch behavior becomes: "either we know you're enrolled and we redirect, or we know you're not enrolled and we issue the session — never 'we couldn't tell so we'll trust you.'"

- **Fix #2 — rate-limit both second-factor endpoints.** Both `POST /login/totp` (web) and `POST /auth/login/totp` (JSON) SHALL extract `State(login_limiter): State<LoginLimiter>` and call `login_limiter.0.check_and_record(ip)` at handler entry, returning `429 Too Many Requests` on budget exhaustion. The shared budget (5 attempts / 15-min window per IP) covers password attempts AND TOTP attempts against the same IP — an attacker can't multiply their budget by switching from `/auth/login` to `/auth/login/totp`.

- **Spec update.** Modify the `session-auth` capability spec to:
  - Add a "TOTP enrollment check fails closed" scenario under the "Login uses 2FA branch when TOTP enrolled" requirement.
  - Modify the existing rate-limiting requirement (in the `rate-limiting` capability) to list both second-factor endpoints alongside `/auth/login` and `/login` in the `login_limiter` set.

- **Tests.**
  - JSON: failing `TotpService::is_enabled` produces 500, not 200 with a session.
  - JSON: 11th attempt on `/auth/login/totp` within the budget window returns 429.
  - Web: parallel coverage for `/login/totp`.
  - Regression: legitimate flows still succeed (TOTP-enrolled member can still complete login when the enrollment query works and they're under the budget).

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `session-auth` — strengthens the "Login uses 2FA branch when TOTP enrolled" requirement with a fail-closed scenario.
- `rate-limiting` — extends the `login_limiter` requirement to cover both second-factor endpoints.

## Impact

- **Code**: ~6–10 lines changed in each of the two login handlers; ~10–15 lines added at each of the two second-factor handlers (limiter extraction + check). Plus tests.
- **Wire shape**: no new routes, no path changes. New possible response: 429 from `/login/totp` and `/auth/login/totp`. 500 instead of 200 on the rare TOTP-service-error path.
- **Risk**: low. The fixes only change behavior in error paths and at-the-budget-limit paths; the happy path is unchanged.
- **Dependency**: none. `secure-enforce-2fa-on-json-login` is already shipped.
- **Operator-visible**: if the TOTP service ever errors (rare), the operator will now see a 500 in logs instead of a silent 2FA-bypass. That's a feature, not a bug.
