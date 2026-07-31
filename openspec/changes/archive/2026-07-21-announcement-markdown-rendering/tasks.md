# Tasks

Turning stored text into HTML on a public surface — the sanitizer whitelist and
the "no raw HTML" render mode are the security controls; call them out in review.

## 1. Dependencies

- [x] 1.1 Add `comrak` and `ammonia` to `Cargo.toml`.

## 2. Shared render pipeline

- [x] 2.1 Add `render_announcement_markdown(md: &str) -> String` (e.g. in
  `src/util/` or a new `src/content/markdown.rs`): comrak with
  `render.unsafe_ = false` (raw HTML NOT rendered) and the GFM strikethrough
  extension, then an `ammonia::Builder` restricted to the safe subset —
  tags `p br em strong del a ul ol li h1..h6 code pre blockquote`, attributes
  `a[href,title,rel]` only, URL schemes `http`/`https`/`mailto`, `rel` forced to
  `noopener nofollow`; everything else (incl. `img`, `script`, event handlers)
  stripped.
- [x] 2.2 Unit tests on the function directly: `<script>` → literal text (not a
  tag); `*i*`/`~~s~~` → `<em>`/`<del>`; `[x](javascript:alert(1))` and a
  `data:` URL → no live link/scheme; `<img>` and `onclick` stripped; a plain
  `https` link preserved with `rel="noopener nofollow"`; stored input string is
  not mutated.

## 3. Portal / admin render

- [x] 3.1 `src/web/portal/announcements.rs:132` — replace
  `escape_html(&announcement.content)` with `render_announcement_markdown(...)`
  injected as safe HTML; drop the now-redundant `whitespace-pre-wrap`.
- [x] 3.2 Apply the same rendering to any other portal/admin surface that shows
  the full announcement body (admin detail view). List/preview surfaces that
  show a truncated snippet SHALL keep plain-text truncation (do not render
  partial HTML).

## 4. Public API + RSS

- [x] 4.1 `src/api/handlers/public.rs::list_announcements` — return a response
  DTO that includes a `content_html` (from the shared pipeline) alongside the
  existing fields; update the `#[utoipa::path]` and `src/api/docs.rs` schema.
- [x] 4.2 RSS builder — set each item `description` to the sanitized rendered
  HTML (CDATA-wrapped).
- [x] 4.3 Tests: `/public/announcements` entry carries a sanitized `content_html`
  (no script/handlers/unsafe schemes); RSS item description carries the rendered
  HTML.

## 5. UI copy

- [x] 5.1 The "Supports Markdown formatting" hints
  (`announcement_new.html:43,45`, `announcement_detail.html:64`) are now
  accurate — keep them (optionally note the supported subset).

## 6. Verify

- [x] 6.1 `openspec validate announcement-markdown-rendering --strict` passes.
- [x] 6.2 `cargo test` green (new + existing announcement/public-feed suites);
  `cargo clippy` clean on touched files.
