# /public/announcements exposes internal announcement fields to anonymous callers

## Summary

`GET /public/announcements` serializes the entire `Announcement` domain struct
to unauthenticated callers, so internal-only fields reach the public marketing
surface — most notably **`created_by`, the author's internal member UUID**, plus
`created_at`, `updated_at`, `announcement_type_id`, `is_public`,
`scheduled_publish_at`, and `scheduled_publish_timezone`. The marketing site
consumes only a small subset (`id`, `title`, `content`, `announcement_type`,
`featured`, `image_url`, `published_at`), so these fields are pure over-share.

This is the announcement analogue of the archived
`public-events-omit-internal-fields` change — but for events, canon
(`public-content-feeds`) explicitly mandated "Other fields SHALL pass through,"
so tightening was a spec change. For announcements, **no canonical requirement
pins the response field set** (`public-content-feeds` →
"/public/announcements returns published public announcements only" constrains
*which* announcements, not *which fields*), so removing internal fields is a
behavior-preserving correction with no spec delta — an **issue**.

## Source location

`src/api/handlers/public.rs` — `list_announcements` returns the raw struct:

```rust
pub async fn list_announcements(
    State(announcement_repo): State<Arc<dyn AnnouncementRepository>>,
) -> Result<Json<Vec<Announcement>>> {
    let announcements = announcement_repo.list_public().await?;
    let published: Vec<Announcement> = announcements
        .into_iter()
        .filter(|a| a.published_at.is_some())
        .collect();
    Ok(Json(published))   // <-- full Announcement struct, incl. created_by etc.
}
```

The iCal/RSS and events paths already use minimal projections; this JSON path
over-shares.

## Why this is harmful

- **Trigger:** any anonymous `GET /public/announcements`.
- **Harm:** `created_by` is an internal identifier (the author's member UUID)
  that should never reach the unauthenticated marketing surface; combined with
  other endpoints it aids enumeration/correlation of members. The scheduling
  fields (`scheduled_publish_at`, `scheduled_publish_timezone`), `is_public`,
  `announcement_type_id`, and the raw timestamps are internal implementation
  detail with no public consumer. Lower severity than an auth bypass, but it is
  unauthorized internal data on a public endpoint — the exact class the
  `public-events` fix removed for events.

## Acceptance criteria (against existing specification)

Behavior-preserving for every legitimate consumer; adds/changes no requirement.

- `/public/announcements` SHALL return a purpose-built PUBLIC PROJECTION
  (`PublicAnnouncement`), not the raw `Announcement` struct — exposing only the
  fields the public surface needs: `id`, `title`, `content`, `announcement_type`,
  `featured`, `image_url`, `published_at` (and `content_html` once
  `announcement-markdown-rendering` lands — see Coordination).
- Internal fields SHALL be omitted: `created_by`, `created_at`, `updated_at`,
  `announcement_type_id`, `is_public`, `scheduled_publish_at`,
  `scheduled_publish_timezone`.
- The published-only + public-only filtering (`public-content-feeds`) is
  unchanged; the RSS feed is unaffected.
- The marketing site is unaffected: it reads only the projected subset. It reads
  `created_at` solely as a fallback in `published_at || created_at`, but a
  published public announcement always has `published_at`, so that fallback is
  already dead — the projected `published_at` covers it.

## Coordination

The `announcement-markdown-rendering` change also introduces a response DTO for
this endpoint (to add `content_html`). Implement a SINGLE `PublicAnnouncement`
projection carrying the safe field set **plus** `content_html`, so whichever
lands second extends the same projection rather than reintroducing the raw
struct. Whichever order they run, the end state is one projection: `id`, `title`,
`content`, `content_html`, `announcement_type`, `featured`, `image_url`,
`published_at`.

## Tasks

- [x] 1. Add a `PublicAnnouncement` projection (mirroring `PublicEvent`) with the
  safe field set above; implement `From<Announcement>`.
- [x] 2. `list_announcements` maps the filtered announcements through the
  projection before `Json(...)`.
- [x] 3. Update the `#[utoipa::path]` response body and `src/api/docs.rs` schema
  to the projection.
- [x] 4. Test: an anonymous `/public/announcements` response carries none of
  `created_by`, `created_at`, `updated_at`, `announcement_type_id`, `is_public`,
  `scheduled_publish_at`, `scheduled_publish_timezone`; the published/public
  filtering and the projected fields are unchanged.
- [x] 5. Verify: `cargo test` (public-feed suites) green; `cargo clippy` clean.
