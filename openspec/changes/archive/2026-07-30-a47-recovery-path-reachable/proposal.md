# a47-recovery-path-reachable

## Why

On 2026-07-29 a member could not sign in. The reconstructed timeline:

```
18:53:08  POST /login           -> 401
19:06:15  POST /login           -> 401
19:06:23  POST /login           -> 401
19:09:22  POST /login           -> 401
19:09:36  POST /login           -> 401     five real failures
19:09:48  POST /login           -> 429     rate limited, correctly
19:10:25  POST /forgot-password -> 429     locked out of his own recovery
```

The rate limiter behaved exactly as specified. That is the problem. `rate-limiting`
canon deliberately puts `/forgot-password` on the same `login_limiter` budget as
`/login`, justified by preventing a stolen-password attacker from brute-forcing
the TOTP step. The consequence nobody wrote down is that **the user most likely to
need a password reset — someone who has just failed five logins — is precisely
the user who can no longer request one.** Forgetting your password locks you out
of the mechanism for handling a forgotten password.

He escaped only by waiting out the window. The reset he eventually completed was
his fifth attempt at it.

A second defect surfaced in the same timeline: `POST /reset-password` returned
`200` on **five** separate attempts, including ones that plainly did not result in
a usable password, because a rejected reset renders an error page with HTTP 200.
Neither the member nor the operator could distinguish "your password was changed"
from "your password was refused" — not from the screen, not from the proxy log.

## What Changes

- **Password recovery gets its own rate-limit budget**, separate from the
  credential budget. Failed logins SHALL NOT consume a member's ability to request
  a reset. Recovery remains rate-limited — it sends email, so it is abusable — but
  on its own bucket with its own limit.
- **The TOTP second factor keeps sharing the credential budget.** That sharing is
  the part of today's rule with a real attack behind it (post-password brute force
  of a six-digit code) and it is not relaxed. The change separates *recovery* from
  *credentials*, not everything from everything.
- **`POST /reset-password` returns a status that reflects what happened.** A
  rejected reset — bad token, expired token, already-consumed token, password
  failing policy — SHALL NOT return `200`. A successful one continues to.
- **The rendered page keeps saying what it says now.** Enumeration-safety is
  unchanged; this is about the status code being honest to logs and monitoring,
  not about revealing more to the caller.

## Impact

- **Spec:** MODIFIED `rate-limiting` (the credential-flow requirement splits
  recovery onto its own limiter), MODIFIED `password-management` (the reset
  requirement gains a status-honesty rule).
- **Code:** a `recovery_limiter` on `AppState` alongside the existing two;
  `src/web/templates/reset.rs` switches limiter and returns non-200 on rejection;
  its background cleanup task registered like the others.
- **Interaction with `a45`:** with auth logging in place, a recovery rejection
  will be visible as an event *and* as a non-200 status. Either alone would have
  shortened this incident; both make it trivial.
- **Not a security regression:** recovery stays limited, TOTP stays on the shared
  credential budget, and no response body becomes more revealing. What changes is
  that a locked-out member can still reach the one door built for them.
- **Deferred:** account-level (rather than IP-level) lockout policy, and an
  operator-visible "this account is currently limited" indicator. Both are real,
  neither is needed to stop trapping people.
