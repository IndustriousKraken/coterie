# Public-site change notifications

If your organization has its own public website that renders Coterie's public
events and announcements, Coterie can tell it when that content changes instead
of leaving it to discover the change on its next poll.

This is optional and off by default. With no endpoint configured, Coterie sends
nothing, logs nothing, and behaves exactly as it did before this existed.

## Why bother

Polling puts a floor on how wrong the public site can be, and the cost of being
wrong is not symmetric:

- **A stale addition** is a mild annoyance — a new event shows up a few minutes
  late.
- **A stale retraction is a disclosure.** An admin who publishes something by
  mistake and corrects it in seconds finds the public site still serving it
  until the next poll. That case is not always an accident either: a speaker may
  ask years later for a talk to come down, or an organization may need to remove
  someone's name quickly. That is routine editorial work, and it should not
  require shell access to the host.

## Configuration

Two settings, under **Settings → Public Site** in the admin portal:

| Setting | Key | Notes |
|---------|-----|-------|
| Endpoint Url | `public_site.endpoint_url` | Where notifications are POSTed. Must start with `http://` or `https://` — anything else is rejected on save. Empty (the default) disables the feature entirely. |
| Secret | `public_site.secret` | Shared secret the request body is signed with. Encrypted at rest and never rendered back to an admin; to change it, type a new one. |

Both take effect immediately — no restart.

## What a receiver must implement

One endpoint that accepts `POST` and answers **any 2xx**. Coterie treats a 2xx
as delivered and anything else (including a redirect) as a failure.

**Request**

```
POST <your endpoint>
Content-Type: application/json
X-Coterie-Signature: sha256=<hex>

{"kind":"event","id":"6f1e…","action":"updated","sent_at":"2026-08-04T17:21:09.482Z"}
```

- `kind` — `"event"` or `"announcement"`.
- `id` — the item's UUID, as it appears in `/public/events` and
  `/public/announcements`.
- `action` — `"updated"` or `"deleted"`. Treat `"updated"` as "re-read this
  item"; it covers creation, edits, publication, and withdrawal alike.
- `sent_at` — RFC 3339. Useful for rejecting stale replays; nothing depends on
  it.

**Verify the signature before acting on anything.** The receiving site publishes
and withdraws content on the strength of these messages, so an unauthenticated
endpoint lets any caller on the internet drive that. Compute
`HMAC-SHA256(secret, raw_request_body)`, hex-encode it, prefix `sha256=`, and
compare against the header — over the **raw body bytes**, before any JSON
parsing or re-serialization, and with a constant-time comparison.

**The payload carries no item content, deliberately.** Not the title, not the
body, not the location, not the image — just enough to identify what to re-read.
Read the content from `GET /public/events` or `GET /public/announcements`, which
already apply the organization's visibility rules. That is what makes disclosure
structurally impossible rather than carefully avoided: with an identifier alone,
your site cannot render anything it was not already entitled to fetch, whatever
it does with the message.

So the handler is roughly:

1. Verify the signature. Reject if it does not match.
2. Answer 2xx immediately — do the work asynchronously if fetching is slow.
3. Re-read the item from the public API.
4. Present if it is there; remove it from your site if it is not.

Step 4 is the whole point. An item missing from the public API has been
withdrawn, and your copy has to go with it.

**Timeouts.** Every attempt is bounded at 5 seconds. A withdrawal is delivered
synchronously from the admin's action so the result can be shown to them, which
means a slow receiver makes an admin wait. Answer fast.

## Keep your poller

**This does not replace your reconciliation.** Push is a latency optimization
over polling, not a replacement for it. Coterie does not queue, retry, or
persist notifications: a dropped one is gone, and a receiver that has removed
its poller has also removed its only means of noticing that it is now silently
wrong. What push buys you is that the reconciling sweep can run hourly instead
of every few minutes, because it is a backstop rather than the primary
mechanism.

Bulk operations lean on that backstop directly: editing a whole recurring series
rewrites many occurrences at once and deliberately does not emit one
notification per row.

## What withdrawal cannot do

**It cannot recall anything a third party already fetched.** A search engine,
scraper, or social platform that already has a copy keeps it. The mitigation is
that the window between publishing and withdrawing is short, not that publishing
can be undone. Do not plan around a guarantee that does not exist.

## When something fails

A failed withdrawal never rolls back the withdrawal — the item is withdrawn in
Coterie either way; what failed is telling your site, and reverting would widen
the inconsistency rather than close it. Coterie tells the admin plainly that the
public site was not updated.

Every event and announcement detail page carries a **Resend to public site**
button (shown only when an endpoint is configured). It sends that one item's
current state and reports the result. It works whether or not the item is
currently public — resending a withdrawn item's state is exactly how a missed
withdrawal is repaired — and it depends on no other part of this feature
working, so it is also the right tool after a misconfiguration or an outage.
