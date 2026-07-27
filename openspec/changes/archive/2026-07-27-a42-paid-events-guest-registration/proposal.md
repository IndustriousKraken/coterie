# a42-paid-events-guest-registration

## Why

`a41-paid-events-member-registration` lets a **member** pay for an event. But the
motivating cases for a paid event — a public workshop, a ticketed talk, a class
open to the community — are exactly the ones where the paying attendee is *not* a
member yet. An org that can only sell seats to existing members cannot use a paid
event to reach anyone new, which is most of the point.

This change opens paid registration to the public: a **guest price** on the event,
a **Coterie-hosted public registration page** whose URL can be pasted into
Discord, a social post, or a newsletter, and a guest attendee record so a
non-member can hold a seat.

It depends on a41 and reuses its state machine wholesale — seat-claim-before-
checkout, payment-status-driven seat release, refund-releases-seat,
refund-before-delete. Nothing about that lifecycle is re-litigated here; a guest
seat is the same seat with a different payer.

### Security framing (why this change carries security requirements)

This adds an **unauthenticated, money-moving, publicly-reachable endpoint** — the
same shape as `/public/signup` and `/public/donate`, which are precisely the
endpoints that were hit with card-testing abuse (62 fake accounts, each with a
Stripe customer and a failed charge, cleaned up in July 2026). Shipping a third
one without the same protections would re-open a hole that was just closed.

So the protections are first-class requirements, not follow-ups:

- **Card testing.** An open "charge this card" endpoint is a free oracle for
  validating stolen card numbers. Mitigated by the bot challenge (Turnstile) and
  the per-IP `money_limiter`, in that documented order.
- **Registration spam / capacity squatting.** Claiming seats without paying can
  deny a real attendee a seat. Mitigated by rate limiting plus a41's rule that a
  seat stops being held once its payment leaves `Pending`.
- **Information disclosure.** The public page must not become a read hole into
  members-only events, so only `Public`-visibility events are registerable and
  the page renders only already-public fields.
- **Cross-account writes.** A guest supplying a member's email must not get a row
  written into that member's account (see the design note below — this is where
  guest registration deliberately diverges from public donation).

## What Changes

- **Guest price and guest eligibility as two separate fields.** New
  `guest_price_cents` (`NOT NULL DEFAULT 0`) for how much a non-member pays, and a
  separate `guest_registration_enabled` boolean for whether non-members may
  register at all. An earlier draft folded both into one nullable price where an
  absent value meant "no public registration" — that made a single column answer
  two unrelated questions and left "the public attends free" indistinguishable
  from "the public may not attend". Split, every configuration is expressible and
  each column means one thing: free for members / paid for the public, paid for
  both at different amounts, or members-only with no public door.
- **Guest attendees.** `event_attendance` gains a surrogate primary key, a
  nullable `member_id`, and guest identity columns, with a CHECK that exactly one
  of member/guest identity is present — mirroring the `payments` table, which
  already relaxed `member_id` this way for public donations in migration 016.
- **A public, shareable registration page** at `GET /events/:id/register`, served
  by Coterie so no per-event marketing-site work is needed, plus the money
  endpoint `POST /public/events/:id/register`.
- **Registerability is independent of price.** An event is publicly registerable
  when it is `Public` and has guest registration enabled — full stop. A guest
  price of `0` means the registration is free, not that it is disabled. This
  keeps three orthogonal questions in three fields: `rsvp_required` (must
  attendees register — already exists), `guest_registration_enabled` (may
  non-members register), and the price (what it costs). Folding price into
  eligibility would make a free workshop with twenty seats unrepresentable, which
  is a real and common offering.
- **The public events feed gains `registration_url` + `guest_price_cents`,**
  populated together and only for registerable events. The URL is resolved
  server-side so a consumer decides whether to offer registration by testing one
  field for presence — never by re-deriving the rule from prices and visibility
  flags, which would duplicate an authorization decision and drift from it.
- **The public page offers login for member pricing** rather than silently
  charging a member the (usually higher) guest price.
- **Guests are never silently attached to member accounts** — the deliberate
  divergence from `/public/donate`, justified in `design.md`.
- **Bot challenge + money rate limit + CSRF exemption** on the new public
  endpoint, matching the existing public-endpoint posture exactly.

## Impact

- **Spec:** `paid-events` gains 9 ADDED requirements. MODIFIED: `bot-challenge`
  (the protected-endpoint list), `rate-limiting` (the `money_limiter` caller
  list), `csrf-protection` (the exempt list plus its justification),
  `domain-types` (`Payer::PublicDonor` is broadened from "public donor" to "any
  non-member payer", which is what it already structurally is),
  `public-content-feeds` (the public event projection gains `registration_url`
  and `guest_price_cents`), and `payment-recording` (see below).
- **On the repeated `payment-recording` amendment:** this change restates a41's
  amendment to the four-entry-points requirement — the one distinguishing
  *recording* a payment from *opening* a `Pending` placeholder — rather than
  relying on a41 having archived first. Guest registration writes a placeholder
  exactly as member registration does, so without the amendment in scope this
  change contradicts canon whenever it is evaluated before a41 lands. The two
  statements are deliberately identical in substance, so whichever archives first
  leaves canon in the same state and the second is a no-op replacement.
- **Code (new):** migration adding `events.guest_price_cents` and rebuilding
  `event_attendance` with a surrogate PK + guest identity + CHECK; a public
  registration page template; `src/web/public_events.rs` (or an extension of the
  existing public surface) for the page and the POST.
- **Code (extend):** the a41 registration service gains a guest path;
  `stripe_client` stamps guest identity into the event-fee session; the roster
  and receipts render guests; CORS/exempt/limiter wiring.
- **Reuse:** a41's entire seat lifecycle, the `BotChallengeVerifier` trait, the
  `money_limiter`, the public-donation guest-payer precedent, receipt emails.
- **Behavior for orgs that set no guest price:** none. No public page exists for
  an event without a guest price.
- **Accepted weakness:** free public registration has no card in front of it, so
  the bot challenge and rate limit are its only abuse controls. A determined
  abuser can consume seats with fabricated emails. Refusing to serve free
  registration would be the safer choice and the wrong one — it makes the
  free-workshop case unrepresentable — so the exposure is stated in the spec and
  admins can release seats from the roster.
- **Deferred:** self-serve guest cancellation (an admin refund releases the seat,
  which covers the case without building an identity-less auth flow), guest
  accounts / converting a guest into a member, guest waitlists, per-event
  registration questions, and requiring email confirmation before a free seat is
  confirmed (the obvious hardening if free-registration spam ever materializes).
