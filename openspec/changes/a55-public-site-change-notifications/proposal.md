# Change: Coterie tells a companion public site when public content changes

## Why

An organization's public website renders Coterie's public events and
announcements. theneontemple.com is one; any Coterie deployment can have one.
Today that site can only learn about a change by polling `/public/events` and
`/public/announcements`, because Coterie emits nothing it could listen to for
anything except creation.

Polling sets an unavoidable floor on how wrong the public site can be, and the
cost of being wrong is not symmetric:

- **A stale addition** is a mild annoyance — a new event is missing for a few
  minutes.
- **A stale retraction is a disclosure.** An admin who publishes something by
  mistake and corrects it in seconds finds that the public site keeps serving it
  until the next poll. Worse, the retraction case is not always an accident: a
  speaker may ask years later for a talk to come down, or an organization may
  need to remove someone's name quickly after events that make association
  harmful. That is routine editorial work, not an emergency drill, and it must be
  self-service.

Right now the only way to force the public site to catch up is for someone with
shell access to log into the host. That is not a feature any organization other
than the one that built it can use.

Coterie already has the seam. `IntegrationEvent` is fanned out by
`IntegrationManager` to registered integrations, and `EventPublished` /
`AnnouncementPublished` already fire. Two gaps stop that seam from serving this:

1. **There is no update or delete signal at all.**
   `event_admin_service.rs::update_one` says so explicitly — *"No integration
   dispatch — updates are silent per existing design."* So an edit, and more
   importantly a retraction, reaches nothing.
2. **`EventPublished` fires on creation, not on becoming public.** It is gated on
   `visibility != AdminOnly` at create time, so an event drafted as members-only
   and later made public emits nothing. The name is misleading: it means
   "created and not admin-only."

## What Changes

- **New variants**: `EventUpdated`, `EventDeleted`, `AnnouncementUpdated`,
  `AnnouncementDeleted`, each carrying post-update state only.

  They deliberately do **not** follow `MemberUpdated { old, new }`. That variant
  carries both snapshots because a consumer computes a delta from it — Discord
  derives role transitions that way — and nothing consumes an event or
  announcement delta. What a consumer does with these is make its own copy match
  current state, which the new value alone describes. A prior snapshot nobody
  reads is content in flight for no purpose, and content in flight is what this
  change most needs to minimize.

  A variant per transition — `Unpublished`, `Republished`, `Rescheduled` — was
  also rejected. Visibility is a field on the state already being carried. One
  update variant covers retraction, late publication, and content edits alike,
  and closes gap 2 without renaming `EventPublished`.

- **Payloads carry no more than the item's own visibility already discloses**,
  enforced at the dispatch site. A `Public` item may carry its public projection;
  a `MembersOnly` item carries only what `/public/events` returns for it; an
  `AdminOnly` item carries identity and visibility alone. And the notification
  that leaves the host carries no item content whatsoever — just kind, id, and
  what happened — so a receiver reads content from the public API or not at all.

  This is a structural choice rather than a careful one. Sending private content
  outward and relying on each recipient to decline to publish it is correct only
  while every consumer, present and future, implements that correctly, and the
  failure is silent when one does not.

- **Dispatch wherever public output can change**, which is the rule rather than a
  list: update, delete, visibility change, and the occurrence-level exceptions
  that add or remove a materialized event from the feed.

- **A public-site notifier**: an integration that posts a signed change
  notification to a configured endpoint. Configuration is a URL and a shared
  secret; absent a URL, nothing is sent and nothing changes for existing
  deployments.

- **Retraction is delivered synchronously and its result shown to the admin.**
  This is the part that matters and the part that is deliberately not on the
  fan-out bus. See below.

- **A per-item "resend to public site" control** on the event and announcement
  admin pages, reporting success or failure. It is the retry when a delivery
  fails, and the escape hatch when the automatic path is broken.

## Why retraction does not ride the integration bus

`integration-events` requires that consumers not block the originating call and
that failures be logged rather than surfaced to the caller. That is the right
design for the traffic it was built for: a missed Discord post is recoverable by
a human reposting it.

It is the wrong design for retraction, because **a security control cannot be
built on a best-effort channel.** If the public site is unreachable for twenty
seconds during a deploy when an unpublish fires, fire-and-forget means the
content stays public and nobody learns of it.

The usual fix is durable delivery — an outbox, retries, acknowledgement — and
that is a queue to build and operate. It is not needed here, because retraction
has a property the other traffic does not: **a human is standing right there,
having just clicked unpublish, waiting for a response.** So the notification is
made as a direct, acknowledged call from the admin action, and its outcome is
rendered in the admin's response. The admin is the retry mechanism, and the
per-item resend control is the retry button.

That keeps the bus exactly as specified for everything it already does, and
introduces no queue. Two channels, chosen by consequence: fire-and-forget for
what can be re-derived by polling, acknowledged for what cannot be left wrong.

## What this does not do

- **It does not replace the companion site's polling.** Blind push has no
  self-healing: one dropped notification and the site drifts silently, and the
  only way to detect drift is a reconciling sweep. What push buys is that the
  sweep can run hourly instead of every few minutes, because it is a backstop
  rather than the primary mechanism.
- **It does not build durable delivery.** No outbox, no retry queue, no ordering
  guarantees. If that is ever needed, the trigger will be a consumer whose
  correctness depends on push alone — which is precisely what keeping the poller
  avoids.
- **It does not recall anything already fetched by a third party.** A platform
  that scraped a preview keeps it. That is a consequence of having published, and
  the mitigation is that the window is short, not that it can be undone.
- **It does not notify on member or payment changes.** This is about public
  content only.
