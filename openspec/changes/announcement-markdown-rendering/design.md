# Design notes — announcement-markdown-rendering

Guidance, not contract. Binding contract is the two spec deltas. The defining
concern is that stored text becomes HTML on a public surface, so the render
pipeline's safety is the design.

## Rendering pipeline (the one shared function)

`render_announcement_markdown(md: &str) -> String`, used by the portal render,
the `/public/announcements` DTO, and the RSS builder — never duplicated.

Two stages, both security-relevant:

1. **Markdown → HTML with `comrak`.** Comrak escapes raw HTML by default
   (`ComrakOptions.render.unsafe_ = false` — leave it off), so admin-typed
   `<script>` becomes literal text, satisfying the retained
   "Script tag in body is escaped" scenario. Enable the GFM **strikethrough**
   extension (and autolink) so `~~x~~` → `<del>`. Do NOT enable `unsafe_`.
2. **Sanitize the HTML with `ammonia`.** Comrak's safe mode neutralizes raw HTML
   but does NOT vet Markdown-produced link schemes (`[x](javascript:…)` would
   yield a `javascript:` href). Ammonia is the authoritative pass: a whitelist
   `Builder` that allows only the safe-subset tags, strips all attributes except
   `href` on `<a>` (and `rel`/`title`), restricts URL schemes to
   `http`/`https`/`mailto`, and — belt and suspenders — is where `<img>`,
   `<script>`, event-handler attributes, and everything else off-list are
   dropped. Add `rel="noopener nofollow"` to links.

Order matters: comrak first (produce structured HTML), ammonia second
(authoritative filter). Ammonia alone is the security boundary; comrak's safe
mode is defense in depth, not the sole control.

**Why two dependencies rather than hand-rolling:** safe Markdown + HTML
sanitization is exactly the security-critical parsing you must not write
yourself. `comrak` and `ammonia` are the maintained, widely-used Rust choices.
This is the "reach for the right dependency" rung, not the "add a dep for a few
lines" anti-pattern.

## Safe subset

Allowed tags: `p br em strong del a ul ol li h1 h2 h3 h4 h5 h6 code pre
blockquote`. Allowed attrs: `a[href,title,rel]` only. Schemes: `http https
mailto`. Everything else stripped. No `img` (announcements have `image_url`); no
`table` in the MVP.

## Render-on-read, not a cached column

Render at read time from stored Markdown. No `content_html` DB column, so: no
migration, no stale cache when the renderer/whitelist is tightened, always
consistent with the current pipeline. Announcement read volume is tiny and
render is cheap; revisit only if profiling ever says so
(`# ponytail: render-on-read; cache if announcement reads ever get hot`).

## API shape

Add `content_html` to a small response DTO for `/public/announcements` (a
serialized wrapper carrying the existing fields plus the rendered field). Keep
raw `content` too, for any consumer that wants source. Update the
`#[utoipa::path]`/`docs.rs` schema.

Note (out of scope): building that DTO is also the natural place to drop the
internal fields the endpoint currently leaks (`created_by`, timestamps,
`announcement_type_id`, `scheduled_publish_*`) — the `public-events` treatment.
Do NOT bundle it here; file it as its own issue so this change stays about
Markdown.

## Portal & RSS

- Portal (`announcements.rs:132`): replace `escape_html(&content)` with
  `render_announcement_markdown(&content)` injected as already-safe HTML. The
  surrounding `whitespace-pre-wrap` on the `<p>` is no longer needed once block
  markup is produced.
- RSS: put the sanitized HTML in the item `description` (CDATA-wrapped), which is
  valid RSS 2.0.

## Marketing site (companion change, separate repo)

In `theneontemple.com` `main.js`: the announcement **modal** renders
`announcement.content_html` via `innerHTML` (safe — server-sanitized) instead of
`content`/`textContent`. **Card previews stay plain text** — truncating
`content_html` could cut mid-tag — so previews truncate the raw `content` and
`escapeHtml` it as today. Sequence after Coterie ships `content_html`.
