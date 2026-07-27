# Tasks

Depends on `a41-paid-events-member-registration` — the seat lifecycle it defines
is reused as-is. Do not fork a second state machine for guests; a guest seat is
the same seat with a different payer.

This change opens an unauthenticated money endpoint. Tasks in section 5 are
security controls, not polish — review them against the abuse that hit
`/public/signup` in July 2026.

## 1. Storage

- [ ] 1.1 Migration: `ALTER TABLE events ADD COLUMN guest_price_cents INTEGER NOT
  NULL DEFAULT 0` and `ADD COLUMN guest_registration_enabled BOOLEAN NOT NULL
  DEFAULT 0`. Two columns, two questions — do NOT encode "no public registration"
  as a null price.
- [ ] 1.2 Same migration: rebuild `event_attendance` with a surrogate
  `id TEXT PRIMARY KEY`, `member_id` relaxed to nullable, and
  `guest_name` / `guest_email` columns. Preserve the `ON DELETE CASCADE` FKs, the
  `payment_id` column and `PendingPayment` status from a41, and `reminder_sent_at`.
- [ ] 1.3 Same migration: `CHECK` that exactly one identity is present —
  `(member_id IS NOT NULL AND guest_email IS NULL) OR (member_id IS NULL AND
  guest_email IS NOT NULL)`. Mirrors the `payments` table's public-donation CHECK
  from migration 016.
- [ ] 1.4 Same migration: `UNIQUE(event_id, member_id)` and
  `UNIQUE(event_id, guest_email)`. These are the DB-level guarantee behind
  one-seat-per-identity; do not rely on the service check alone.

## 2. Domain & repository

- [ ] 2.1 `EventAttendance`: `member_id` becomes `Option<Uuid>`, add
  `guest_name`/`guest_email`. Model the identity as a sum type rather than three
  loose optionals, matching how `Payer` handles the same problem.
- [ ] 2.2 `Event`: `guest_price_cents: i64` and `guest_registration_enabled: bool`,
  plus a `publicly_registerable()` predicate (`visibility == Public &&
  guest_registration_enabled` — price is NOT part of it) so the 404 rule has one
  home instead of being re-derived per call site.
- [ ] 2.3 Repository: seat claim, roster, and refund lookups accept a guest
  identity. The capacity count from a41 is row-based and needs NO change —
  confirm with a test rather than editing it.
- [ ] 2.4 `find_event_fee_payment` gains a by-guest-email lookup for the
  double-charge guard.

## 3. Registration service

- [ ] 3.1 Guest path on the a41 service: same ordering (claim seat → payment +
  session → release on failure), guest identity on the row and on the payment as
  `Payer::PublicDonor`.
- [ ] 3.1b Free guest path (`guest_price_cents == 0`): claim and confirm the seat
  immediately, no payment row and no Checkout session — the same short-circuit
  a41 uses for a free member registration. Protections still apply in full.
- [ ] 3.2 Guest double-charge guard keyed on `(event_id, guest_email)`.
- [ ] 3.3 Do NOT look the guest email up against members. Add a test asserting a
  guest using a member's email produces a guest row and writes nothing into that
  member's account.
- [ ] 3.4 Bound and validate guest name/email at the boundary.

## 4. Public surface

- [ ] 4.1 `GET /events/:id/register` — public page. 404 unless
  `publicly_registerable()`. Render only public fields; never the roster.
- [ ] 4.2 Show member price + login link when a member price exists, so a member
  is not silently charged the guest price.
- [ ] 4.3 Sold-out state: say so, render no form.
- [ ] 4.4 `POST /public/events/:id/register` — the money endpoint.
- [ ] 4.5 Guest confirmation email on seat confirmation: event details, plus a
  receipt only when money was actually paid. Skip silently when no email provider
  is configured.
- [ ] 4.6 `PublicEvent` projection gains `registration_url: Option<String>` and
  `guest_price_cents: Option<i64>`, populated together and only when
  `publicly_registerable()`. Emit a resolved absolute URL — consumers must not
  re-derive registerability. Update the OpenAPI schema.

## 5. Protections (security controls)

- [ ] 5.1 Wire `money_limiter` to the new POST, applied BEFORE the bot challenge.
- [ ] 5.2 Wire the `BotChallengeVerifier`, fail-closed, matching
  `/public/signup`'s behavior and log outcomes.
- [ ] 5.3 Add `POST /public/events/:id/register` to `CSRF_EXEMPT_PATHS` with the
  written justification the exempt-list requirement demands.
- [ ] 5.4 Confirm the CORS allowlist covers the endpoint for the marketing origin.
- [ ] 5.5 Verify a rate-limited or challenge-failed request claims NO seat and
  creates NO payment row — the rejection must happen before any state is written.

## 6. Admin

- [ ] 6.1 Guest price field + "allow non-members to register" checkbox on the
  admin event forms. Zero stores as zero, same as the member price. Enabling guest
  registration at a zero price is a VALID, supported combination (free workshop) —
  do not reject it.
- [ ] 6.2 Roster shows guest name/email and visually distinguishes guests from
  members.
- [ ] 6.3 At-the-door, comp, release-stuck-seat, and refund all work on guest rows
  and audit identically.

## 7. Tests

- [ ] 7.1 A `MembersOnly` event and a nonexistent event id produce
  indistinguishable 404s from the public page.
- [ ] 7.2 An event with `guest_registration_enabled = false` is not publicly
  registerable, and is distinguishable in storage from one enabled at a zero
  price — assert both, since collapsing them is the bug this design avoids.
- [ ] 7.2b Free registerable event: page served, seat confirmed with no payment
  row, and `/public/events` emits a `registration_url` with
  `guest_price_cents = 0`. A zero price must not suppress the URL.
- [ ] 7.2c A show-up event (guest registration disabled) emits a null
  `registration_url` — the common case for this org's weekly events.
- [ ] 7.3 Guest happy path: form → seat held → webhook → `Registered` +
  confirmation email.
- [ ] 7.4 Guest abandonment frees the seat (reuses a41's expiry path).
- [ ] 7.5 Guest and member seats compete for the same capacity.
- [ ] 7.6 Concurrent duplicate guest email yields exactly one seat (exercise the
  UNIQUE constraint, not just the service guard).
- [ ] 7.7 Guest using a member's email does not touch that member's account.
- [ ] 7.8 Rate-limited request is rejected without consulting the provider and
  without claiming a seat; challenge-failed request likewise writes nothing.
- [ ] 7.9 Guest refund releases the seat.
