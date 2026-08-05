//! Shared Markdown → sanitized-HTML rendering for admin-authored bodies:
//! announcement `content` and event `description`.
//!
//! Security-critical: both are admin-authored text that becomes HTML on
//! the member portal AND the public marketing surface, so this is the sole
//! path from stored Markdown to displayed HTML — one safe subset, one
//! scheme allow-list, one image exclusion, so a change to what is safe
//! reaches every content type at once. Two stages, both security-relevant:
//!
//! 1. **comrak, raw-HTML passthrough OFF** (`render.unsafe_ = false`, the
//!    default — do NOT enable it). An admin who types `<script>` gets
//!    literal text, not a live tag.
//! 2. **ammonia — the authoritative filter.** comrak's safe mode
//!    neutralizes raw HTML but does NOT vet Markdown-produced link schemes
//!    (`[x](javascript:…)` would yield a `javascript:` href). ammonia is a
//!    whitelist: only the fixed safe-subset tags survive, `a[href,title,rel]`
//!    are the only attributes, URL schemes are limited to http/https/mailto,
//!    and `rel="noopener nofollow"` is forced onto links. This is where
//!    `<img>`, `<script>`, event-handler attributes, and unsafe schemes are
//!    dropped.
//!
//! Order matters: comrak first (structure), ammonia second (authoritative
//! filter). ammonia alone is the security boundary; comrak's safe mode is
//! defense in depth.

use std::collections::{HashMap, HashSet};

use comrak::{markdown_to_html, Options};

/// Render admin-authored Markdown (an announcement body, an event
/// description) into a sanitized HTML safe-subset string. The input `md`
/// is never mutated. See the module docs for the security model.
pub fn render_markdown(md: &str) -> String {
    let mut options = Options::default();
    // GFM strikethrough (`~~x~~` → <del>) and autolink.
    options.extension.strikethrough = true;
    options.extension.autolink = true;
    // Raw/embedded HTML is NOT rendered as live markup — `render.unsafe_`
    // defaults to false; leave it off. `render.escape` makes comrak escape
    // raw HTML to literal text (an admin's `<script>` shows as text) rather
    // than omitting it as `<!-- raw HTML omitted -->`, so the "script tag is
    // displayed as literal text" scenario holds.
    options.render.escape = true;
    let rendered = markdown_to_html(md, &options);

    let allowed_tags: HashSet<&str> = [
        "p",
        "br",
        "em",
        "strong",
        "del",
        "a",
        "ul",
        "ol",
        "li",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "code",
        "pre",
        "blockquote",
    ]
    .into_iter()
    .collect();

    // Only `<a>` keeps attributes: `href` and `title`. `rel` is force-set
    // by `link_rel` below — ammonia forbids also listing it here.
    let mut tag_attributes: HashMap<&str, HashSet<&str>> = HashMap::new();
    tag_attributes.insert("a", ["href", "title"].into_iter().collect());

    let url_schemes: HashSet<&str> = ["http", "https", "mailto"].into_iter().collect();

    // ponytail: Builder rebuilt per call; render-on-read volume is tiny.
    // Make it a LazyLock<Builder<'static>> only if announcement reads ever
    // get hot (the string literals above are already 'static).
    ammonia::Builder::default()
        .tags(allowed_tags)
        .generic_attributes(HashSet::new())
        .tag_attributes(tag_attributes)
        .url_schemes(url_schemes)
        .link_rel(Some("noopener nofollow"))
        .clean(&rendered)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_script_becomes_literal_text_not_a_tag() {
        let out = render_markdown("<script>alert(1)</script>");
        assert!(
            !out.contains("<script"),
            "raw <script> must not survive as a tag: {out}"
        );
        // comrak escapes it; the visible text is preserved.
        assert!(out.contains("alert(1)"), "script text should remain: {out}");
    }

    #[test]
    fn italic_and_strikethrough_render_to_safe_html() {
        let out = render_markdown("*italic* and ~~struck~~");
        assert!(out.contains("<em>italic</em>"), "italic → <em>: {out}");
        assert!(out.contains("<del>struck</del>"), "strike → <del>: {out}");
    }

    #[test]
    fn bold_and_bulleted_list_render() {
        let out = render_markdown("**bold**\n\n- one\n- two");
        assert!(
            out.contains("<strong>bold</strong>"),
            "bold → <strong>: {out}"
        );
        assert!(out.contains("<ul>"), "list → <ul>: {out}");
        assert!(out.contains("<li>one</li>"), "list item rendered: {out}");
    }

    #[test]
    fn javascript_link_scheme_is_neutralized() {
        let out = render_markdown("[click](javascript:alert(1))");
        assert!(
            !out.contains("javascript:"),
            "javascript: scheme must be stripped: {out}"
        );
    }

    #[test]
    fn data_link_scheme_is_neutralized() {
        let out = render_markdown("[x](data:text/html,<h1>hi</h1>)");
        assert!(
            !out.contains("data:"),
            "data: scheme must be stripped: {out}"
        );
    }

    #[test]
    fn img_and_event_handlers_are_stripped() {
        // Markdown image syntax → comrak <img>; ammonia drops it (no live img).
        let img = render_markdown("![alt](https://example.com/x.png)");
        assert!(
            !img.contains("<img"),
            "Markdown <img> must be stripped: {img}"
        );

        // Raw HTML with an event handler is escaped to inert text by comrak
        // (render.escape) and never reaches the DOM as a live attribute: the
        // anchor is shown as escaped text (`&lt;a …`), not a live `<a onclick>`.
        let handler = render_markdown("<a href=\"#\" onclick=\"steal()\">x</a>");
        assert!(
            !handler.contains("<a "),
            "raw anchor must not become a live tag: {handler}"
        );
        assert!(
            handler.contains("&lt;a"),
            "raw anchor should render as escaped literal text: {handler}"
        );
    }

    #[test]
    fn https_link_preserved_with_forced_rel() {
        let out = render_markdown("[site](https://example.com)");
        assert!(
            out.contains("href=\"https://example.com\""),
            "https href preserved: {out}"
        );
        assert!(
            out.contains("rel=\"noopener nofollow\""),
            "rel forced to noopener nofollow: {out}"
        );
    }

    #[test]
    fn input_is_not_mutated() {
        let src = String::from("# Heading\n\n*text* with <script>x</script>");
        let before = src.clone();
        let _ = render_markdown(&src);
        assert_eq!(src, before, "stored source must be untouched by rendering");
    }
}
