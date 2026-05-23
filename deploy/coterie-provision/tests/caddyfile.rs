use coterie_provision::caddyfile::{has_coterie_marker, render_caddyfile, COTERIE_MARKER};

const FIXTURE: &str = include_str!("fixtures/caddyfile_example.txt");

#[test]
fn portal_only_drops_marketing_block() {
    let out = render_caddyfile(FIXTURE, "portal.acme.io", None);
    assert!(out.contains("portal.acme.io {"));
    assert!(!out.contains("coterie.example.com"));
    // Marketing section is gone.
    assert!(!out.contains("example.com, www.example.com"));
    assert!(!out.contains("Public marketing"));
}

#[test]
fn portal_and_marketing_render_both() {
    let out = render_caddyfile(FIXTURE, "portal.acme.io", Some("acme.io"));
    assert!(out.contains("portal.acme.io {"));
    assert!(out.contains("acme.io, www.acme.io {"));
    // Original example domains must be substituted out of the active
    // site blocks. (Operator-facing commentary may still mention them.)
    assert!(!out.contains("coterie.example.com"));
    assert!(!out.contains("example.com, www.example.com"));
}

#[test]
fn marker_prepended_and_idempotent_on_rerender() {
    let out = render_caddyfile(FIXTURE, "portal.acme.io", None);
    assert!(has_coterie_marker(&out));
    let twice = render_caddyfile(&out, "portal.acme.io", None);
    assert_eq!(twice.matches("coterie-managed").count(), 1);
}

#[test]
fn weird_hyphenated_domains() {
    let out = render_caddyfile(FIXTURE, "my-portal.acme-corp.io", Some("acme-corp.io"));
    assert!(out.contains("my-portal.acme-corp.io {"));
    assert!(out.contains("acme-corp.io, www.acme-corp.io {"));
}

#[test]
fn deep_subdomain() {
    let out = render_caddyfile(FIXTURE, "members.portal.acme.io", None);
    assert!(out.contains("members.portal.acme.io {"));
}

#[test]
fn marker_string_is_what_we_expect() {
    assert!(COTERIE_MARKER.starts_with('#'));
    assert!(COTERIE_MARKER.contains("coterie-managed"));
}
