# Tasks

## 1. Optional session resolution

- [ ] 1.1 Add a helper that resolves a `CookieJar` to `Option<(CurrentUser,
  SessionInfo)>` without rejecting. `src/api/middleware/auth.rs::authenticate`
  already does this work but is wired to reject; factor the lookup — session
  cookie → `auth_service.validate_session` → `member_repo.find_by_id` → status
  check — so both the rejecting middleware and the new optional caller share one
  implementation. Do not duplicate the cookie name or the status predicate.
- [ ] 1.2 Every failure mode returns `None`, not an error: no cookie, unparseable
  cookie, `validate_session` error or `Ok(None)`, member not found, repository
  error, or a status the portal would block. The registration pages fall back to
  the anonymous rendering on `None`, so an error here must not surface. Treating a
  blocked status as `None` is deliberate: `/portal/api/events/:id/rsvp` is gated to
  Active members, so offering an Expired member the member action would render a
  button that redirects instead of registering. They get the guest form, which
  works.
- [ ] 1.3 The helper must not be added as a router layer. These two routes live on
  the anonymous web surface; adding a layer there would change the tier that
  `auth-middleware-tiers` fixes for every other anonymous route.

## 2. Event registration page

- [ ] 2.1 `src/web/templates/event_register.rs`: call the helper before building
  the template. Keep the `publicly_registerable()` check and its `404` first and
  unchanged — a session must not widen what is visible.
- [ ] 2.2 When a member resolves, build the context with `BaseContext::for_member`
  (it carries the CSRF token the authenticated action needs) rather than
  `for_anon`, and add a template field carrying the member's registration state
  for this event: not registered, already holding a seat.
- [ ] 2.3 Price shown to a signed-in member is `member_price_cents`, rendered `0`
  as "Free" the same way the guest price is. The "Members pay X — log in" line is
  redundant for them and should not render.
- [ ] 2.4 `templates/events/register.html`: branch on the signed-in state. Signed
  in and unregistered → the authenticated action posting to
  `/portal/api/events/{id}/rsvp` with the CSRF token, no name/email inputs, no
  Turnstile widget and no Turnstile script tag. Signed in and already holding a
  seat → a plain "You're already registered" panel with no action. Anonymous →
  today's markup, byte-for-byte.
- [ ] 2.5 The sold-out branch takes precedence over all of the above, as it does
  now.

## 3. Class registration page

- [ ] 3.1 `src/web/templates/class_register.rs` and
  `templates/events/class_register.html`: the same treatment against the series
  member pass price, posting to `/portal/api/series/{id}/enroll` — the route the
  portal class button already uses. Already-enrolled renders the no-action panel.
- [ ] 3.2 Share the signed-in/registration-state shape between the two pages
  rather than writing it twice — they are the same page at two scopes and already
  share `RegisterPageQuery`.

## 4. Return path for anonymous visitors

- [ ] 4.1 The "log in to register at the member price" link becomes
  `/login?redirect=<url-encoded current path>`.
- [ ] 4.2 The login handlers currently filter the post-authentication destination
  with `url.starts_with("/portal/") && !url.contains("..")`, in both
  `login_handler` and the TOTP completion handler. `/events/<id>/register` fails
  that filter and would be silently dropped. Extend the predicate to also admit
  paths matching the two registration pages, keeping it an allow-list and keeping
  the `..` rejection. Put the predicate in one function used by both call sites —
  it is currently written out twice, which is how the two copies drift.
- [ ] 4.3 `login_page`'s already-authenticated branch redirects to
  `/portal/dashboard` and ignores `?redirect=` entirely. Make it honor the same
  allow-listed destination, so a member who arrives with a live session is not
  bounced away from the event they were opening.

## 5. Tests

- [ ] 5.1 Signed-in member opening a paid event's registration page: response
  contains the member price and the authenticated action, and contains no
  `name=` / `email=` input and no Turnstile site key.
- [ ] 5.2 Anonymous visitor on the same event: response is unchanged from today —
  guest form present, guest price shown, member price and login link shown.
- [ ] 5.3 Request carrying a session cookie that does not validate renders the
  anonymous page, including the bot challenge. Assert this for both a syntactically
  bogus cookie value and a session row that has expired.
- [ ] 5.4 Signed-in member who already holds a seat sees the already-registered
  panel and no registration action.
- [ ] 5.5 Signed-in member requesting a non-publicly-registerable event gets `404`,
  matching the anonymous response for the same id.
- [ ] 5.6 The class page repeats 5.1 and 5.4 at series scope.
- [ ] 5.7 Login destination allow-list: `/events/<uuid>/register` and
  `/classes/<uuid>/register` are honored; `https://evil.example/`,
  `//evil.example`, and `/portal/../events/x/register` all fall back to the
  default destination. Cover both `login_handler` and the TOTP completion path,
  since the filter exists in both.
- [ ] 5.8 Regression guard: assert the anonymous rendering still reaches the guest
  endpoint `/public/events/:id/register`, so the signed-in branch cannot quietly
  become the only path.
