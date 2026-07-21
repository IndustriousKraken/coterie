# announcement-markdown-rendering

## Why

The admin announcement form tells authors "Markdown formatting is supported"
(`templates/admin/announcement_new.html:43,45`, `announcement_detail.html:64`),
but nothing renders it. The portal escapes the body
(`src/web/portal/announcements.rs:132`, `escape_html`) and the marketing site
prints it as `textContent` (`main.js:128`), so `*italic*` and `~~strikethrough~~`
show as literal punctuation. The UI promises a feature that does not exist.

We make the promise true rather than delete it: announcement bodies are authored
in Markdown and rendered as a **sanitized** HTML safe-subset everywhere they
display. This is net-new rendering behavior, so it is a **change**, not an issue.

Canon already anticipates exactly this: `admin-announcements` →
"User-supplied announcement content is escaped on render" says *"Any opt-in to
render-as-HTML SHALL be limited to admin-curated, sanitized content ... so a
stored XSS via announcements is prevented."* Announcements are admin-authored,
so a **sanitized** Markdown pipeline is the opt-in that requirement contemplates.
Because that requirement's display behavior changes (escape → sanitized render),
it is MODIFIED here; the new safe-render contract is ADDED as its own capability.

### Security framing

Turning stored text into HTML — on the member portal AND the **public** marketing
surface — is the whole risk. Even though authors are admins (lower trust than
anonymous submitters), rendering to the public gets full defense in depth:

- **Raw/embedded HTML is never emitted as live markup.** The Markdown renderer
  runs with raw-HTML passthrough disabled (an admin typing `<script>` sees it as
  literal text, preserving the existing canon scenario).
- **Output is sanitized to a fixed safe subset** with a whitelist; disallowed
  tags/attributes are stripped, and link URL schemes are restricted to
  http/https/mailto (no `javascript:`/`data:`). Markdown inline images are not
  emitted — the announcement's dedicated `image_url` is the only image surface.
- **Rendered once, server-side.** The public API ships a server-rendered
  `content_html`, so the browser runs no Markdown parser and makes no
  sanitization/trust decision of its own.

## What Changes

- Announcement `content` is treated as **Markdown**; the stored raw value is
  unchanged (source of truth). A single shared server-side renderer converts it
  to sanitized HTML at read time.
- **Portal / admin views** render the sanitized HTML instead of `escape_html`.
- **`GET /public/announcements`** gains a server-rendered sanitized
  `content_html` field alongside the raw `content`; **`GET /public/feed/rss`**
  item descriptions carry the sanitized HTML.
- **Marketing site (companion change, `theneontemple.com` repo — not governed by
  this spec):** the modal renders `content_html` via `innerHTML` (safe: already
  sanitized) instead of `textContent`; card previews stay plain-text
  (truncate the raw `content`, escaped) to avoid truncating mid-tag.
- The three "Supports Markdown" UI hints become true.

## Impact

- **Spec:** `admin-announcements` — 1 MODIFIED requirement (display path). New
  capability `announcement-markdown` — 2 ADDED requirements (render contract +
  public `content_html`/RSS).
- **Deps (new):** `comrak` (Markdown; raw-HTML rendering left disabled — its
  default) and `ammonia` (HTML sanitizer / URL-scheme whitelist). Safe Markdown
  rendering is security-critical parsing — the correct move is a maintained
  library, not a hand-rolled subset.
- **Code:** a shared `render_announcement_markdown(&str) -> String` (comrak →
  ammonia); portal render site (`announcements.rs:132`) uses it; a
  `content_html` on the `/public/announcements` response DTO and the RSS builder;
  update the OpenAPI schema for the new field.
- **Tests:** the retained `<script>`-is-literal scenario; `*italic*`/`~~strike~~`
  render to `<em>`/`<del>`; `javascript:` link stripped; `<img>`/`onclick`
  stripped; `content_html` present and sanitized in the public JSON and RSS.
- **Out of scope (noted, not bundled):** `/public/announcements` currently
  serializes the full `Announcement` struct, leaking internal fields
  (`created_by`, timestamps, `announcement_type_id`, `scheduled_publish_*`) to
  anonymous callers — the same class as `public-events-omit-internal-fields`.
  That deserves its own issue (a `PublicAnnouncement` projection), not this
  change. This change only ADDS `content_html`.
- **Deferred (v2):** tables, task lists, and other extended Markdown; a WYSIWYG
  editor.
