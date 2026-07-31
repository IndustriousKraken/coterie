# a48-login-page-signup-link

## Why

The portal login page renders a "create account" link pointing at
`/public/signup` (`templates/auth/login.html:14`). That route is a **POST-only
JSON API** consumed by the marketing site — there is no GET handler. Following
the link issues a GET, gets a 405, and the browser downloads the JSON error body
as a file. A member reported exactly that during the 2026-07-29 lockout, while
trying to work out why they could not get in.

The link is broken in a way that cannot be repaired by pointing it somewhere
better inside Coterie, because **Coterie has no self-service signup page and
should not grow one**. Account creation happens on the organization's own public
site, which posts to `/public/signup`; the portal is for people who already have
accounts. So the fix is not "find the right internal URL" — it is to stop
advertising a page that does not exist.

Simply deleting the link is tempting and slightly wrong: for an org whose public
site does have a join page, offering the way there is genuinely useful to someone
who landed on the login screen by mistake.

## What Changes

- **A new optional org setting `org.signup_url`**, empty by default.
- **The link renders only when that setting is non-empty**, and points at it.
  Empty setting means no link at all — which is the correct default, because a
  fresh install has no public signup page and should advertise none.
- **`org.website_url` is deliberately not reused.** It already exists, but it
  answers a different question — "where is the org's site" — and its stock value
  is the placeholder `https://example.com`. Pointing a "create account" link at an
  org's homepage sends someone to hunt for a join form, and pointing it at
  `example.com` on a fresh install is the same class of broken link this change
  exists to remove. One setting, one question.

## Impact

- **Spec:** MODIFIED `admin-settings` — the org settings list gains `org.signup_url`
  with its default-empty semantics and the render-only-when-set rule.
- **Code:** migration inserting the setting; `templates/auth/login.html` renders
  the link conditionally; the login handler supplies the value to the template.
- **Behavior on upgrade:** the setting arrives empty, so the broken link
  disappears for every deployment until an operator fills it in. That is the
  correct migration: a missing link is strictly better than one that downloads a
  405 body.
- **For The Neon Temple specifically:** set it to
  `https://theneontemple.com/join/` and the link works properly for the first
  time.
- **Deferred:** a Coterie-hosted signup page. Not wanted — signup lives on the
  public site by design, and the portal deliberately has no anonymous
  account-creation surface.
