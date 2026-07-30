# Tasks

Two independent fixes to the same recovery path: it must be reachable when a
member is locked out, and it must tell the truth about whether it worked.

Do not "simplify" by putting recovery back on the credential budget. The sharing
was deliberate once; this change is the record of what it cost.

## 1. Separate recovery budget

- [x] 1.1 Add `recovery_limiter: RateLimiter` to `AppState` alongside
  `login_limiter` and `money_limiter`, with its own per-IP bucket. Start at the
  same 5-per-15-minutes shape unless there is a reason to differ; the point is
  independence, not a looser number.
- [x] 1.2 Add the `FromRef` impl so handlers can extract it, mirroring how
  `LoginLimiter` and `MoneyLimiter` are already wired.
- [x] 1.3 Register its background `cleanup()` task in `main.rs` on the same cadence
  as the other limiters, so the map does not grow unboundedly.
- [x] 1.4 `src/web/templates/reset.rs`: `POST /forgot-password` switches from
  `login_limiter` to `recovery_limiter`.
- [x] 1.5 Leave `/login`, `/auth/login`, `/login/totp`, `/auth/login/totp` on
  `login_limiter`. The TOTP sharing is the part with a real attack behind it.

## 2. Honest reset status

- [x] 2.1 `POST /reset-password`: return a non-`200` status when the reset did not
  change the password — invalid, expired, or already-consumed token, or a password
  failing `validate_password`.
- [x] 2.2 Keep the rendered body exactly as it is. This task changes status codes
  only; making the page more specific would be an enumeration regression.
- [x] 2.3 Check the HTMX/redirect behavior on the reset page still works with the
  new status — if the form posts normally and re-renders, a 4xx with an HTML body
  is fine, but verify rather than assume.
  Verified: `templates/auth/reset_password.html` and `auth/forgot_password.html`
  post natively (`<form method="POST" action=...>`), carry no `hx-post`, and
  `layouts/base.html` sets no `hx-boost` — htmx never sees these submissions, so
  the browser renders the 4xx HTML body as it does the 200. (`hx-post` is only on
  `/login`, `/login/totp`, `/setup`, `/logout`, none of which changed status.)

## 3. Tests

- [x] 3.1 Exhaust `login_limiter`, then assert `POST /forgot-password` still
  succeeds. This is the regression that trapped a real member — it deserves a
  named test.
- [x] 3.2 Exhaust `recovery_limiter`, then assert `POST /login` is still accepted:
  the independence runs both ways.
- [x] 3.3 Assert TOTP still shares the credential budget (the existing
  cross-surface test must keep passing).
- [x] 3.4 A reset with an already-consumed token returns non-`200`.
- [x] 3.5 A reset with an over-length password returns non-`200` (pairs with
  `a46`).
- [x] 3.6 A successful reset returns success and the new password authenticates.
