# Tasks

## 1. Enum variants

- [x] 1.1 `src/integrations/mod.rs`: add `EventUpdated(Event)`,
  `EventDeleted(Event)`, `AnnouncementUpdated(Announcement)`, and
  `AnnouncementDeleted(Announcement)` to `IntegrationEvent`. Post-update state
  only — no `{ old, new }` pair. `MemberUpdated` carries both because Discord
  computes role transitions from the delta; nothing consumes an event or
  announcement delta, and a prior snapshot nobody reads is content in flight for
  no purpose.
- [x] 1.2 Update every consumer match — Discord, UniFi, admin-alert email. None
  has a default arm, which is the point: the compiler names each place that has to
  decide. Discord should ignore the new variants for now rather than gaining
  edit-spam behavior nobody asked for; say so in a comment at each arm so the
  silence is a decision rather than an oversight.
- [x] 1.3 Do NOT rename `EventPublished`. Its meaning ("created and not
  AdminOnly") is now documented in canon, and `EventUpdated` covers the
  became-public transition it misses. A rename is churn across every consumer for
  no behavior change.

## 2. Dispatch sites

- [x] 2.1 `src/service/event_admin_service.rs::update_one` — currently carries the
  comment "No integration dispatch — updates are silent per existing design."
  Replace that design and the comment: dispatch `EventUpdated` after
  the repository write succeeds, using the pre-update row already loaded there.
- [x] 2.2 Event delete path — dispatch `EventDeleted` after the write.
- [x] 2.3 Announcement update and delete paths in
  `src/service/announcement_admin_service.rs` — the same treatment. Note it
  already dispatches `AnnouncementPublished` from three places; keep those.
- [x] 2.4 Per-occurrence exceptions (cancel, override, restore) in the recurring
  path change what the public feed contains for that occurrence. Dispatch for
  them too.
- [x] 2.5 Suppress the dispatch when nothing publicly observable changed: an item
  `AdminOnly` both before and after, or an edit confined to fields the public
  projection omits. The rule in canon is about observable output, so implement the
  predicate once and call it from each site rather than repeating a visibility
  check per call.
- [x] 2.6 Reduce the dispatched payload to what the item's own visibility already
  discloses, at the dispatch site: `Public` may carry its public projection,
  `MembersOnly` carries only what `/public/events` returns for it (sanitized title,
  no description/location/image), `AdminOnly` carries identity and visibility only.
  Reuse the existing public-projection sanitizer rather than writing a second one —
  two implementations of "what may leave" will drift, and the drift is silent.
- [x] 2.7 Enforce it at dispatch, never by asking consumers to be careful. Sending
  private content outward and trusting each recipient not to publish it holds only
  while every current and future consumer implements that correctly, and fails
  silently when one does not.
- [x] 2.8 Never dispatch on a failed write. Dispatch after the repository call
  returns success, never before or in parallel.

## 3. The notifier

- [x] 3.1 New settings: a public-site endpoint URL and a shared secret. The
  secret is a sensitive setting — follow how existing integration credentials are
  stored and masked, do not invent a second scheme.
- [x] 3.2 Absent an endpoint, the whole capability is inert: no attempt, no error,
  no log noise. Deployments without a companion site must see zero change.
- [x] 3.3 Validate the endpoint URL is http(s) on write, the same way
  `org.signup_url` is scheme-checked before it reaches an `href`.
- [x] 3.4 Sign the request body with the shared secret so the receiver can verify
  origin. The receiver publishes and withdraws content based on these messages, so
  an unauthenticated endpoint would let any caller drive that.
- [x] 3.5 Payload carries kind, id, and what happened — and **no item content at
  all**. The receiver reads current state from the public API, which already
  applies visibility rules. This makes disclosure structurally impossible rather
  than carefully avoided: with an identifier alone, a receiver cannot render
  anything it was not already entitled to fetch, whatever it does with the
  message.
- [x] 3.6 Bound every attempt with a short timeout.

## 4. Two delivery paths

- [x] 4.1 Register the notifier as an `Integration` so it receives creates and
  ordinary updates through the existing fan-out. Loss there is acceptable — the
  companion site's own reconcile catches it.
- [x] 4.2 Withdrawal — delete, or a visibility/publication change that removes the
  item from the public API — does NOT go through the fan-out. Call the notifier
  directly from the admin action and surface the result in the response.
- [x] 4.3 Do not weaken `integration-events`' "consumers do not block the
  originating call" requirement to accommodate 4.2. The withdrawal path is
  deliberately not an integration event; that requirement stays exactly as it is
  for the traffic it governs. A future reader will be tempted to "fix" the
  inconsistency by moving withdrawal onto the bus — the code comment must say why
  that would be wrong.
- [x] 4.4 A failed withdrawal notification does not roll back the withdrawal.
  The item is withdrawn in Coterie either way; reverting would widen the
  inconsistency, not close it.
- [x] 4.5 No outbox, no retry queue, no persisted delivery state. The human in the
  loop is the retry mechanism and the resend control is the retry button. Anything
  more is an operational component bought for a case that does not need it.

## 5. Admin controls

- [x] 5.1 Per-item resend control on the event detail and announcement detail
  admin pages, reporting outcome. Per item, not a global rebuild: an admin using
  it wants an answer about the one item they are looking at.
- [x] 5.2 Available whether or not the item is currently public — resending a
  withdrawn item's state is precisely how a missed withdrawal gets repaired.
- [x] 5.3 Hidden entirely when no endpoint is configured.
- [x] 5.4 The withdrawal response tells the admin plainly whether the public site
  was updated, and where to retry when it was not. This is the whole point of the
  synchronous path; a silent failure here is the defect being fixed.

## 6. Tests

- [x] 6.1 `update_one` dispatches `EventUpdated` carrying the post-update state.
- [x] 6.2 Delete dispatches `EventDeleted`; announcement update and delete
  dispatch their variants.
- [x] 6.3 Members-only → `Public` dispatches `EventUpdated` and NOT
  `EventPublished`. This is the gap that made the old design unable to see a
  late publication.
- [x] 6.4 `Public` → `AdminOnly` dispatches an update carrying no title,
  description, location, or image.
- [x] 6.5 A `MembersOnly` item's dispatch carries no more than `/public/events`
  returns for it. Assert against the projection's own output so the two cannot
  drift apart unnoticed.
- [x] 6.6 No notification payload, for an item of any visibility, contains item
  content. This is the assertion that keeps the disclosure property structural.
- [x] 6.7 An `AdminOnly`-to-`AdminOnly` edit dispatches nothing.
- [x] 6.8 A repository failure dispatches nothing.
- [x] 6.9 Cancelling an occurrence of a public series dispatches.
- [x] 6.10 With no endpoint configured: no HTTP attempt on any of the above, and
  every admin action behaves identically to today. Assert the absence of the
  attempt, not just the absence of an error.
- [x] 6.11 Withdrawal with a reachable endpoint reports success to the admin;
  withdrawal with an unreachable one reports failure, and the item is still
  withdrawn.
- [x] 6.12 An endpoint that never responds does not hang the admin's request
  beyond the timeout bound.
- [x] 6.13 The signature verifies against the configured secret, and a body
  altered in transit fails verification.
- [x] 6.14 The secret does not appear in the settings page HTML or in log output.
- [x] 6.15 Resend sends for an item that is currently withdrawn.
- [x] 6.16 The resend control is absent when unconfigured.

## 7. Documentation

- [x] 7.1 Document the two settings and what a receiver must implement, in the
  deployment docs where other integrations are described. A companion site is
  something another organization may want to build; the contract needs to be
  readable without reading the source.
- [x] 7.2 State plainly that this does not replace the receiver's own
  reconciliation. Push is a latency optimization over polling, not a replacement:
  a dropped notification leaves the receiver silently wrong, and only a
  reconciling sweep finds that. A receiver that deletes its poller because push
  exists has removed its only means of detecting drift.
- [x] 7.3 State that withdrawal cannot recall anything a third party already
  fetched. The mitigation is a short window, not an undo, and nobody should plan
  around a guarantee that does not exist.
