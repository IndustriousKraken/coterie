//! a48: the login page's create-account link is gated on `org.signup_url`.
//!
//! The defect: the link was hardcoded to `/public/signup`, a POST-only JSON
//! API. A browser following it as a GET got a 405 and downloaded the error
//! body as a file — which is what a locked-out member hit on 2026-07-29.
//! Coterie hosts no signup page of its own, so an unconfigured deployment
//! must advertise none.
//!
//! Run with: cargo test --test login_signup_link_test

use std::{fs, path::Path};

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use sqlx::SqlitePool;
use tower::ServiceExt;

mod common;
use common::{build_app_state, fresh_pool, merged_router};

const SIGNUP_URL: &str = "https://theneontemple.com/join/";

async fn harness() -> (SqlitePool, Router) {
    let pool = fresh_pool().await;
    let state = build_app_state(pool.clone()).await;
    state
        .admin_exists_observed
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let app = merged_router(state);
    (pool, app)
}

async fn login_page(app: &Router) -> String {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/login")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

async fn set_signup_url(pool: &SqlitePool, value: &str) {
    let affected = sqlx::query("UPDATE app_settings SET value = ? WHERE key = 'org.signup_url'")
        .bind(value)
        .execute(pool)
        .await
        .expect("update org.signup_url")
        .rows_affected();
    assert_eq!(affected, 1, "migration 045 must have inserted the setting");
}

#[tokio::test]
async fn the_setting_ships_empty_and_the_link_is_absent() {
    let (pool, app) = harness().await;

    let stock: String = sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
        .bind("org.signup_url")
        .fetch_one(&pool)
        .await
        .expect("org.signup_url exists");
    assert_eq!(stock, "", "a fresh install advertises no signup page");

    let body = login_page(&app).await;
    assert!(
        !body.contains("create a new account"),
        "an unconfigured deployment must render no create-account link"
    );
    assert!(
        !body.contains("/public/signup"),
        "the POST-only signup API must never appear on the login page"
    );
}

#[tokio::test]
async fn a_configured_signup_url_is_linked() {
    let (pool, app) = harness().await;
    set_signup_url(&pool, SIGNUP_URL).await;

    let body = login_page(&app).await;
    assert!(
        body.contains(&format!(r#"href="{SIGNUP_URL}""#)),
        "the link must point at the configured page; body was: {body}"
    );
    assert!(body.contains("create a new account"));
    assert!(
        !body.contains("/public/signup"),
        "configuring a real page must not resurrect the broken target"
    );
}

/// Whitespace-only is the same as unset: an operator who cleared the field
/// by typing a space still gets no link rather than `href=" "`.
#[tokio::test]
async fn a_blank_signup_url_is_treated_as_unset() {
    let (pool, app) = harness().await;
    set_signup_url(&pool, "   ").await;

    let body = login_page(&app).await;
    assert!(!body.contains("create a new account"));
}

/// The setting is admin-written but rendered to anonymous visitors, and
/// HTML-escaping does nothing to a URL *scheme*. A non-http(s) value reads
/// as unset, so `javascript:` can never become a click-to-execute link.
#[tokio::test]
async fn a_non_http_signup_url_renders_no_link() {
    for value in ["javascript:alert(1)", "data:text/html,<b>x", "example.com"] {
        let (pool, app) = harness().await;
        set_signup_url(&pool, value).await;

        let body = login_page(&app).await;
        assert!(
            !body.contains("create a new account"),
            "{value} must render no link at all; body was: {body}"
        );
        assert!(
            !body.contains(value),
            "{value} must not reach the page in any form; body was: {body}"
        );
    }
}

/// The actual defect class: any template linking a browser at the POST-only
/// signup API. A grep-style assertion stops it returning by a different route.
#[test]
fn no_template_links_a_browser_at_the_signup_api() {
    let mut offenders = Vec::new();
    for file in html_templates(Path::new("templates")) {
        let body = fs::read_to_string(&file).expect("read template");
        if href_values(&body).any(|v| v.starts_with("/public/signup")) {
            offenders.push(file.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "/public/signup is a POST-only JSON API — a browser following an href \
         to it gets a 405 and downloads the body. Offending templates: {offenders:?}"
    );
}

fn html_templates(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).expect("read templates dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            out.extend(html_templates(&path));
        } else if path.extension().is_some_and(|e| e == "html") {
            out.push(path);
        }
    }
    out
}

/// Quoted `href` attribute values, both quote styles. Deliberately naive —
/// it only has to see literal targets, which is what the defect was.
fn href_values(body: &str) -> impl Iterator<Item = &str> {
    body.match_indices("href=").filter_map(|(i, _)| {
        let rest = &body[i + "href=".len()..];
        let quote = rest.chars().next()?;
        if quote != '"' && quote != '\'' {
            return None;
        }
        rest[1..].split(quote).next()
    })
}
