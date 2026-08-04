# Change: The shareable registration pages recognize a signed-in member

## Why

`GET /events/:id/register` and `GET /classes/:id/register` are the URLs an
organizer pastes into Discord or a newsletter. Both build their context with
`BaseContext::for_anon()` and never look at the session cookie. A member who is
already signed in to the portal — the common case, because the same Discord
message that carries the link is read by members — sees the guest form, types
their name and email into it, and is charged the guest price.

The page does try to prevent this: when the member price differs it renders
"Members pay $X. Log in to register at the member price instead." That line is
doing all of the work, and it is losing. It relies on the visitor noticing a
sentence above the form, and it offers them a login link that (a) they do not
need, because they are already logged in, and (b) drops them on `/login` with no
way back — a member who follows it lands on their dashboard and has to find the
event again by hand.

The failure is silent and expensive to undo: the guest seat is a real Stripe
charge attached to a guest attendee row, not to the member's account. Fixing it
after the fact means a partial refund or a release-and-re-register, both of which
are manual admin work per person. This has already happened in production.

Nothing here changes what a genuinely anonymous visitor sees. The whole change is
that a request carrying a valid member session stops being treated as anonymous.

## What Changes

- The two shareable registration pages resolve an existing member session. When
  the visitor is an authenticated member, the page charges the member price and
  registers them as themselves, through the same authenticated path
  `/portal/events` already uses — not the guest endpoint.
- A signed-in member is not asked for a name and email the system already has,
  and is not shown a bot challenge, because the request is session-authenticated
  rather than an anonymous money endpoint.
- A signed-in member who already holds a seat is told so, instead of being shown
  a button that looks like it will charge them again.
- An invalid, expired, or absent session is treated as anonymous — the guest form,
  unchanged. Session resolution failing must not take the page down for guests.
- For visitors who really are anonymous, the "log in to register at the member
  price" link carries a return path back to the registration page, so the round
  trip is one click instead of a manual hunt through the events list.

Non-goals: the guest registration flow, guest pricing, capacity, and the seat and
payment lifecycles are untouched. This change only decides *which existing path*
a given visitor is offered.
