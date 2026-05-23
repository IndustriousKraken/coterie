/// Render `/etc/caddy/Caddyfile` from the shipped `Caddyfile.example`.
///
/// Substitutions performed:
///   * The literal `coterie.example.com` → `portal_domain`.
///   * The literal `example.com, www.example.com` site block →
///     `marketing_domain, www.marketing_domain`. If `marketing_domain`
///     is `None`, the second site block is removed entirely.
///
/// We also prepend a `# coterie-managed` marker so the wizard can later
/// detect that it produced this file and decide whether to clobber it
/// on a re-run.
pub fn render_caddyfile(
    template: &str,
    portal_domain: &str,
    marketing_domain: Option<&str>,
) -> String {
    let mut working = template.replace("coterie.example.com", portal_domain);

    if let Some(md) = marketing_domain {
        let needle = "example.com, www.example.com";
        let replacement = format!("{md}, www.{md}");
        working = working.replace(needle, &replacement);
        prepend_marker(working)
    } else {
        let trimmed = strip_marketing_block(&working);
        prepend_marker(trimmed)
    }
}

pub const COTERIE_MARKER: &str = "# coterie-managed (do not edit by hand)";

fn prepend_marker(body: String) -> String {
    if body.starts_with(COTERIE_MARKER) {
        body
    } else {
        format!("{COTERIE_MARKER}\n{body}")
    }
}

/// Returns true if the file's first non-blank line contains the Coterie
/// marker comment we write at render time.
pub fn has_coterie_marker(contents: &str) -> bool {
    contents
        .lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.contains("coterie-managed"))
        .unwrap_or(false)
}

/// Strip the marketing site block. We find the section header comment
/// and remove everything from there to the end of the matching block.
fn strip_marketing_block(input: &str) -> String {
    // The Caddyfile.example structures the marketing site under a
    // section header comment followed by a site block opening with
    // `example.com, www.example.com {`. We delete from the header line
    // (or from the site-block start if no header) through the closing
    // brace, plus any trailing whitespace.
    let lines: Vec<&str> = input.lines().collect();
    let header_re = "Public marketing";
    let site_open = "example.com, www.example.com {";

    let mut start = None;
    let mut end = None;
    for (i, line) in lines.iter().enumerate() {
        if start.is_none() && line.contains(site_open) {
            start = Some(i);
        }
    }
    if let Some(open_idx) = start {
        // Walk backwards from open_idx to find a contiguous block of
        // comment / blank lines preceding the site block; treat those
        // as the section header.
        let mut header_start = open_idx;
        for i in (0..open_idx).rev() {
            let l = lines[i].trim_start();
            if l.starts_with('#') || l.is_empty() {
                header_start = i;
                if l.contains(header_re) {
                    // Walk further up to capture the `# ----` rule
                    // line above the header, if present.
                    for j in (0..i).rev() {
                        let lj = lines[j].trim_start();
                        if lj.starts_with('#') {
                            header_start = j;
                        } else {
                            break;
                        }
                    }
                    break;
                }
            } else {
                break;
            }
        }
        // Walk forward from open_idx to find the matching close brace.
        let mut depth = 0i32;
        for (i, line) in lines.iter().enumerate().skip(open_idx) {
            for ch in line.chars() {
                if ch == '{' {
                    depth += 1;
                } else if ch == '}' {
                    depth -= 1;
                }
            }
            if depth <= 0 && i > open_idx {
                end = Some(i);
                break;
            }
        }
        if let Some(close_idx) = end {
            let mut kept: Vec<&str> = Vec::with_capacity(lines.len());
            kept.extend(&lines[..header_start]);
            kept.extend(&lines[close_idx + 1..]);
            // Re-stitch with newlines and trim trailing blank lines so
            // we don't end with three newlines.
            let mut out = kept.join("\n");
            // Preserve trailing newline if the original had one.
            if input.ends_with('\n') {
                out.push('\n');
            }
            return out;
        }
    }
    input.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> &'static str {
        include_str!("../tests/fixtures/caddyfile_example.txt")
    }

    #[test]
    fn portal_only_drops_marketing_block() {
        let out = render_caddyfile(fixture(), "portal.acme.io", None);
        assert!(out.contains("portal.acme.io {"));
        assert!(!out.contains("coterie.example.com"));
        // Marketing block removed entirely.
        assert!(!out.contains("example.com, www.example.com"));
        assert!(!out.contains("Public marketing"));
    }

    #[test]
    fn portal_and_marketing_render_both() {
        let out = render_caddyfile(fixture(), "portal.acme.io", Some("acme.io"));
        assert!(out.contains("portal.acme.io {"));
        assert!(out.contains("acme.io, www.acme.io {"));
        // No example.com left in actual site blocks or directives. We
        // allow the literal in unrelated commentary if it slips in
        // because the template is operator-facing docs.
        assert!(!out.contains("coterie.example.com"));
        assert!(!out.contains("example.com, www.example.com"));
    }

    #[test]
    fn marker_prepended() {
        let out = render_caddyfile(fixture(), "portal.acme.io", None);
        assert!(has_coterie_marker(&out));
        // Idempotent: re-rendering keeps a single marker.
        let twice = render_caddyfile(&out, "portal.acme.io", None);
        assert_eq!(
            twice.matches("coterie-managed").count(),
            1,
            "marker must not be duplicated on re-render"
        );
    }

    #[test]
    fn weird_but_valid_domain_with_hyphens() {
        let out = render_caddyfile(fixture(), "my-portal.acme-corp.io", Some("acme-corp.io"));
        assert!(out.contains("my-portal.acme-corp.io {"));
        assert!(out.contains("acme-corp.io, www.acme-corp.io {"));
    }

    #[test]
    fn deep_subdomain() {
        let out = render_caddyfile(fixture(), "members.portal.acme.io", Some("acme.io"));
        assert!(out.contains("members.portal.acme.io {"));
    }
}
