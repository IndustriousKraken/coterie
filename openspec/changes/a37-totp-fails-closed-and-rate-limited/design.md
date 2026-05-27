## Context

Two TOTP-surface weaknesses surfaced in post-merge review of `secure-enforce-2fa-on-json-login`:

1. The `is_enabled` enrollment check in both password-step handlers swallows errors via `.unwrap_or(false)`. The pattern was carried from the original web handler into the new JSON handler verbatim, so this change touches both.
2. The two second-factor endpoints (`/login/totp` and `/auth/login/totp`) have no rate limit. The first-factor endpoints (`/login` and `/auth/login`) share `login_limiter` (a `LoginLimiter(pub RateLimiter)` extracted via `State`); the second-factor endpoints simply don't pull it in.

Both are small fixes; bundling them is correct because they're the same threat-model surface (2FA bypass via post-password attack vectors) and changing each handler exactly once is cheaper than two PRs that touch the same files.

## Goals / Non-Goals

**Goals:**
- A TOTP-enrollment query failure SHALL result in 500, never a silent 2FA-skip. Symmetrically on web and JSON.
- The two second-factor endpoints SHALL participate in the same per-IP rate limit that the first-factor endpoints already use.
- Test coverage demonstrates both fixes, including the regression baseline that legitimate flows still work.

**Non-Goals:**
- Per-pending-session attempt cap (e.g., "5 TOTP guesses per pending row, then the row is consumed"). That's a finer-grained mitigation worth doing separately if the IP-shared budget proves insufficient — it requires a schema change and atomic increment, which is out of scope for this fix.
- Per-member TOTP attempt counter. Same reasoning.
- Differentiating the TOTP rate-limit budget from the password rate-limit budget. Sharing them is the right default (an attacker can't multiply their budget by switching paths); if a legitimate user gets locked out of TOTP because they exhausted the budget on password retries, the 15-min window unblocks them.
- Changing the pending-login TTL or any other adjacent surface.

## Decisions

### D1. Fail-closed via `?` with context

```rust
let totp_enabled = totp_service
    .is_enabled(member.id)
    .await
    .context("failed to query TOTP enrollment status")?;
```

The `?` propagates the underlying error up the handler's `Result<Response>` (JSON) or surfaces as a 500 via the `Response` builder (web). Operators see the error in logs; attackers don't get a silent bypass.

Why not retry-with-backoff? Because a TOTP-enrollment check should be cheap (single-row indexed lookup); a failure here means something serious (DB down, pool exhausted) and we don't want the handler papering over it. Surface fast, alert operators, let the user retry.

### D2. Apply existing `LoginLimiter` to both second-factor endpoints

Both `/login/totp` and `/auth/login/totp` extract:

```rust
State(login_limiter): State<LoginLimiter>,
```

At handler entry, before any other work:

```rust
let ip = state::client_ip(&headers, settings.server.trust_forwarded_for());
if !login_limiter.0.check_and_record(ip) {
    return Err(AppError::TooManyRequests);  // JSON
    // OR: return (StatusCode::TOO_MANY_REQUESTS, ...).into_response();  // Web
}
```

Note: the web `/login/totp` handler currently uses `Json(payload): Json<LoginTotpRequest>` and doesn't extract `headers: HeaderMap`. Add the headers extractor so `client_ip` works.

### D3. Shared budget is correct

The `LoginLimiter` is a single `RateLimiter` instance shared across all auth endpoints that call it. Pre-this-change: `/login` + `/auth/login` share the budget. Post-this-change: `/login/totp` + `/auth/login/totp` also share it. An attacker hammering `/auth/login` with bad passwords and then switching to `/auth/login/totp` with a stolen password sees the same 5-attempt budget across both. That's the right model: the cost to the attacker is bounded regardless of which path they use.

Edge case: a legitimate user fails 4 password attempts, finally succeeds, then enters one wrong TOTP code. The next failed TOTP attempt would hit budget exhaustion. The user waits 15 minutes and tries again. Mildly annoying but not broken — and the failure surface is "user fat-fingers their TOTP" which is rare in practice.

If this edge case ever bites a real org, the right next move is the per-pending-session attempt cap from Non-Goals — but only if observed.

### D4. Error type for the 429

JSON: `AppError::TooManyRequests` → maps to 429 via the existing `IntoResponse` impl. (Confirmed it exists since `/auth/login` already returns it.)

Web: return `(StatusCode::TOO_MANY_REQUESTS, ...)` with a JSON-or-HTML body matching the existing `/login` handler's pattern. The web `/login` handler returns `Json(LoginResponse { ... })` with `error` set — match that shape for consistency.

### D5. Tests

The existing test file `tests/api_login_totp.rs` covers the JSON side. Extend it with two new tests:

- `json_login_returns_500_if_totp_enrollment_check_errors` — inject a `TotpService` that returns Err for `is_enabled`; submit password-only login; assert 500, no session cookie, no pending_login cookie.
- `json_login_totp_returns_429_after_budget_exhausted` — submit 5 bad TOTP codes from the same IP (each fails with 401); 6th attempt returns 429.

For the web side, equivalent tests in `tests/web_login_totp.rs` (or wherever the parallel test file lives). If no such file exists yet, create it following the same harness pattern as `api_login_totp.rs`.

## Risks / Trade-offs

- **Risk**: a legitimate user's IP hits the rate limit because of mixed password + TOTP attempts. → Mitigation: 15-min window auto-recovers; documented edge case.
- **Risk**: surfacing 500 on TOTP-enrollment-check failures means a transient DB blip turns into a visible user-facing error. → Acceptable: this is what 500s exist for. The alternative (silent 2FA bypass) is unacceptable.
- **Risk**: the test that injects a failing `TotpService` requires a fake/mock — verify the existing test harness supports this (it should, since `TotpService` is constructed in the harness already; just need to swap the implementation).
- **Trade-off**: bundling two fixes in one change vs. two changes. They're on the same surface, same files, same threat model, same review session — bundling is correct. Each fix is small enough that splitting adds bureaucracy without value.

## Migration Plan

Single PR.

1. Update `src/api/handlers/auth.rs::login` — change `.unwrap_or(false)` to `?` with context.
2. Update `src/web/templates/auth.rs::login_handler` — same change.
3. Update `src/api/handlers/auth.rs::login_totp` — extract `LoginLimiter`, check at entry, return 429 on exhaust.
4. Update `src/web/templates/auth.rs::login_totp_handler` — extract `LoginLimiter` + `HeaderMap`, check at entry, return 429 on exhaust.
5. Add the four new tests (two JSON, two web).
6. Update the `session-auth` spec to add the fail-closed scenario.
7. Update the `rate-limiting` spec to list both second-factor endpoints in `login_limiter` callers.
8. `cargo test`, `cargo clippy --deny warnings`, `cargo fmt --check`.
