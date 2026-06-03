//! Render tests for the a40 "update available" banner.
//!
//! The decision of *whether* to show the banner is a pure function
//! (`service::update_check::banner_for`) unit-tested in that module.
//! These tests close the loop on the template: a `BaseContext` carrying
//! an injected banner value — the shape the cached latest-stable release
//! produces — actually renders the banner markup in a page that extends
//! `layouts/base.html`, and a `BaseContext` without one renders nothing.
//!
//! We render `MemberDashboardTemplate` because it is the simplest portal
//! page that extends the shared layout; the banner lives in the layout,
//! so any portal page exercises it.

use askama::Template;
use chrono::TimeZone;
use coterie::{
    domain::MemberStatus,
    service::update_check::{banner_for, LatestRelease, UpdateBanner},
    web::{
        portal::{dashboard::MemberDashboardTemplate, MemberInfo},
        templates::BaseContext,
    },
};
use uuid::Uuid;

fn member_info() -> MemberInfo {
    MemberInfo {
        id: Uuid::parse_str("11111111-2222-3333-4444-555555555555").unwrap(),
        username: "jdoe".to_string(),
        full_name: "Jane Doe".to_string(),
        email: "jane@example.com".to_string(),
        status: MemberStatus::Active,
        membership_type: "Regular".to_string(),
        joined_at: chrono::Utc
            .with_ymd_and_hms(2025, 9, 12, 14, 30, 0)
            .unwrap(),
        dues_paid_until: Some(chrono::Utc.with_ymd_and_hms(2026, 3, 1, 12, 0, 0).unwrap()),
    }
}

fn render_with(base: BaseContext) -> String {
    MemberDashboardTemplate {
        base,
        member: member_info(),
    }
    .render()
    .expect("render dashboard")
}

/// An injected cache value: latest stable `v1.4.0`, running `v1.3.0`,
/// admin session. This is what the daily check would have cached.
fn injected_banner() -> Option<UpdateBanner> {
    let cached = LatestRelease {
        tag: "v1.4.0".to_string(),
        notes_url: "https://github.com/IndustriousKraken/coterie/releases/tag/v1.4.0".to_string(),
    };
    banner_for(true, Some("v1.3.0"), Some(&cached), None)
}

#[test]
fn admin_behind_a_newer_release_sees_the_banner() {
    let banner = injected_banner().expect("admin behind a newer stable gets a banner");
    let base = BaseContext {
        is_admin: true,
        update_banner: Some(banner),
        update_readme_url:
            "https://github.com/IndustriousKraken/coterie/blob/v1.3.0/README.md#update".to_string(),
        ..BaseContext::default()
    };

    let html = render_with(base);

    assert!(
        html.contains("Update available:"),
        "banner heading should render"
    );
    assert!(html.contains("v1.4.0"), "the newer tag should render");
    assert!(
        html.contains("releases/tag/v1.4.0"),
        "release-notes link should render"
    );
    assert!(
        html.contains("README.md#update"),
        "how-to-update link should render"
    );
    assert!(
        html.contains("id=\"update-banner\""),
        "banner element (and its dismiss script) should render"
    );
    assert!(
        html.contains("update_dismissed"),
        "the client-side dismiss script should be present"
    );
}

#[test]
fn no_banner_value_renders_no_banner_markup() {
    // The default context has `update_banner: None` — what a member, a
    // dev build, or an up-to-date instance produces.
    let html = render_with(BaseContext::default());
    assert!(
        !html.contains("Update available:"),
        "no banner should render without an injected value"
    );
    assert!(
        !html.contains("id=\"update-banner\""),
        "no banner element should render without an injected value"
    );
}

#[test]
fn member_never_gets_a_banner_even_when_behind() {
    // A non-admin session with the same newer cache: the decision
    // function returns None, so nothing renders.
    let cached = LatestRelease {
        tag: "v1.4.0".to_string(),
        notes_url: "https://example.test/r/v1.4.0".to_string(),
    };
    let banner = banner_for(false, Some("v1.3.0"), Some(&cached), None);
    assert!(banner.is_none(), "members get no banner from the decision");

    let base = BaseContext {
        is_admin: false,
        update_banner: banner,
        ..BaseContext::default()
    };
    let html = render_with(base);
    assert!(!html.contains("Update available:"));
}
