# Design — a42-paid-events-guest-registration

## What is reused, unchanged, from a41

A guest seat is the same seat. Every lifecycle rule from a41 applies as written,
because they are all expressed in terms of the attendance row and its linked
payment, not in terms of the payer being a member:

- claim the seat **before** minting the Checkout session;
- a `PendingPayment` row holds a seat only while its payment is `Pending`, so
  abandonment self-releases through `handle_expired_session`;
- completion confirms the seat via webhook, never via the `success_url`;
- refund releases the seat; deleting a paid event refunds first.

This change adds a payer shape and a public door. It does not add a second state
machine — a second one would inevitably drift from the first on exactly the
edge cases (double-pay, race, refund) that cost real money.

## Schema: one attendance table, not two

`event_attendance` is today `PRIMARY KEY (event_id, member_id)` with `member_id`
NOT NULL. Guests need a row without a member.

**Chosen:** rebuild the table with a surrogate `id` primary key, nullable
`member_id`, `guest_name` / `guest_email`, and a CHECK enforcing exactly one
identity:

```sql
CHECK ( (member_id IS NOT NULL AND guest_email IS NULL)
     OR (member_id IS NULL     AND guest_email IS NOT NULL) )
```

plus `UNIQUE(event_id, member_id)` and `UNIQUE(event_id, guest_email)` so the
one-seat-per-identity rule survives concurrency at the database level rather than
only in service code.

This mirrors `payments`, which relaxed `member_id` to nullable with a CHECK for
public donations in migration 016. Matching the precedent means the roster query,
the capacity count, the reminder join, and the refund lookup each stay a single
query over a single table.

**Rejected:** a parallel `event_guest_registrations` table. It would fork every
read path — capacity would become a UNION, the roster a UNION, reminders a UNION
— permanently, to avoid one table rebuild once. SQLite table rebuilds are already
the established pattern here (a41 does one for the status CHECK).

## Guests are NOT matched to member accounts by email

`/public/donate` looks the donor's email up and attaches the donation to a
matching member. Guest registration deliberately **does not**.

The reason is that a donation is a gift to the org — attributing it to the right
member is helpful and harmless. A registration is a **seat with a price**, and
email is unverified input on this endpoint. Auto-matching would mean an anonymous
caller can, by typing a member's address:

- write an attendance row and a payment row into that member's account, and
- decide which price bracket that member's seat was sold at.

Neither is catastrophic (they did pay), but both are cross-account writes driven
by an unauthenticated string, and the feature works fine without them. So a guest
registration stays a guest registration even when the email matches a member.

The cost is a member who registers through the public page pays the guest price.
That is solved in the UI rather than in the data model: the public page shows the
member price alongside the guest price whenever a member price exists, with a
login link. A member who is about to overpay is told so before paying, which is
the honest fix — silently re-pricing based on an unverified email is not.

## `Payer::PublicDonor` is broadened, not renamed

The variant is `PublicDonor { name, email }` and structurally it is already
"non-member payer whose identity we captured for receipts" — exactly a guest
registrant. This change updates the spec's *description* of the variant to say so
and reuses it.

Renaming it to something neutral would be more accurate and was considered, but
it would touch every donation code path to fix a naming nit, in the same change
that opens a public money endpoint. Not worth coupling those risks. If the name
is ever changed, it should be its own mechanical change with no behavior in it.

## "How much" and "whether" are two questions, so they are two columns

`guest_price_cents` is `NOT NULL DEFAULT 0` and `guest_registration_enabled` is a
separate boolean.

The first draft of this change had a single nullable `guest_price_cents` in which
`NULL` meant "the public may not register". That is one column answering two
unrelated questions, and it collapses two genuinely different states —

- the public may attend at no charge, and
- the public may not attend at all

— into the same stored value. It also inherits the `NULL`-as-zero problem the
member price already rejects: `WHERE guest_price_cents = 0` would find no free
event, and range filters would silently omit them.

With the fields split, each column has one meaning, every combination is
representable, and the eligibility question is answerable without inspecting a
price.

## Three questions, three fields

The org this is built for runs a weekly talk that anyone walks into, plus a
handful of workshops a year that need a seat list. Those are different on the
*registration* axis, not the *price* axis — a workshop can be free and still need
twenty named seats.

So:

| Question | Field |
|---|---|
| Must attendees register? | `rsvp_required` *(already exists — reuse it)* |
| May non-members register? | `guest_registration_enabled` |
| What does attendance cost? | `member_price_cents` / `guest_price_cents` (`0` = free) |

An earlier draft made public registerability depend on `guest_price_cents > 0`.
That was the same mistake as using `NULL` for a free price, one level up: it made
one field answer two questions and rendered the free-workshop-with-limited-seats
case unrepresentable. Price and eligibility are now fully orthogonal.

## Public exposure rules

The public page and endpoint exist for an event when **both** hold:

1. `visibility = 'Public'` — a `MembersOnly` or `AdminOnly` event must not become
   readable or registerable through a public URL, and
2. `guest_registration_enabled` — the org has opened the door to non-members.

Anything else is a 404, not a 403: a 403 on a members-only event id confirms the
event exists, which is a small enumeration leak for no benefit.

When the guest price is `0`, registration still happens — it just skips checkout
and confirms the seat immediately, exactly as a member registering for a free
event does.

## Free registration's weaker position, stated plainly

A paid registration has a card in front of it, and a card is friction. A free one
does not, so the bot challenge and the per-IP rate limit are the only controls
standing between an open endpoint and a roster full of fabricated names.

That is genuinely weaker, and the honest thing is to say so rather than to pretend
Turnstile makes it equivalent. It is accepted because the alternative — refusing
to serve free registration — makes a real and common offering unbuildable, and
because the failure mode is recoverable: an admin can see the roster and release
seats, unlike a fraudulent charge which costs money to unwind.

The obvious hardening, if free-registration spam ever actually shows up, is to
require an emailed confirmation link before a free seat is confirmed — the same
pattern `signup-verify-email-before-checkout` already established. It is not built
now because it is speculative, and building it costs a whole verification flow.

## One field decides the marketing site's behavior

`/public/events` emits `registration_url` — an absolute URL, present only when the
event is publicly registerable — plus `guest_price_cents` alongside it.

The alternative was to publish the ingredients (`guest_registration_enabled`,
prices, visibility) and let the marketing site decide. Rejected: whether an event
may be publicly registered is an authorization rule, and a second implementation
of it in JavaScript would drift from the first the moment either changes. The
server already knows the answer, so it emits the answer.

The consumer's whole rule becomes "if `registration_url` is present, show a
Register button." Nothing to keep in sync.

The page renders only fields that are already public for a `Public` event (title,
description, time, location, price, remaining capacity). It does not render the
roster — who is attending is not public information.

## Protection ordering on the money endpoint

`money_limiter` runs **before** the bot-challenge provider, matching the rule
already established for `/public/signup`: a bursting IP must not be able to burn
the org's Turnstile quota. Order is: CORS allowlist → rate limit → bot challenge
→ handler.

The endpoint is CSRF-exempt for the same reason `/public/donate` is — the caller
has no session, so there is no session id to bind a token to. The justification
is recorded in the exempt list itself, as that requirement demands.

## Guest confirmation

On payment confirmation the guest receives an email with the event details and a
receipt, reusing the existing receipt-email path and its "skip silently when no
email provider is configured" rule. That email is the guest's only artifact of
the registration — there is no account for them to log into, which is why
self-serve cancellation is deferred rather than half-built: doing it properly
needs a signed per-registration link, and an admin refund already covers the
real-world case.
