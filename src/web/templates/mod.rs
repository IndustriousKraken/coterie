pub mod auth;
pub mod filters;
pub mod reset;
pub mod setup;
pub mod verify;

use askama::Template;
use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};

use crate::api::middleware::auth::{CurrentUser, SessionInfo};
use crate::auth::CsrfService;

/// Context every page that extends `layouts/base.html` carries.
///
/// Embedding this struct as `pub base: BaseContext` on each template
/// struct keeps the layout-level fields in one place: adding a new
/// global template variable means updating exactly this struct and
/// the layout, not 40 individual page structs.
///
/// `csrf_token` lives here because the layout renders a global
/// `<meta name="csrf-token">` that the HTMX `htmx:configRequest`
/// handler reads to stamp `X-CSRF-Token` on every state-changing
/// request — including the global logout button. Per-page templates
/// that omit the token would silently break logout from that page.
#[derive(Debug, Clone, Default)]
pub struct BaseContext {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub csrf_token: String,
    /// The running version string (e.g. `"v1.2.3 (abc1234)"` or
    /// `"0.1.0-dev (abc1234)"`), rendered in the footer's "about this
    /// version" line on every portal page. See [`crate::version`].
    pub version: String,
    /// The GitHub release page URL when this is a real release build
    /// (`crate::version::release_tag()` is `Some`); `None` for dev
    /// builds, which have no release page and render the version with
    /// no link.
    pub release_url: Option<String>,
    /// The admin-only "update available" banner, or `None`. Populated by
    /// [`BaseContext::for_member`] from the cached latest stable release
    /// when the session is an admin and the running release build is
    /// behind it. `None` for members, dev builds, and up-to-date
    /// instances — see [`crate::service::update_check::banner_for`].
    pub update_banner: Option<crate::service::update_check::UpdateBanner>,
    /// Version-pinned link to the README "Update" steps, used by the
    /// update banner's "How to update" link. Empty unless a banner is
    /// shown, and only referenced from inside the banner markup.
    pub update_readme_url: String,
}

impl BaseContext {
    /// Build a base context for an authenticated portal page. Mints a
    /// fresh CSRF token bound to the active session — every authenticated
    /// page renders with a usable token, so HTMX state-changing actions
    /// (including the global logout button in the layout) work from any
    /// page.
    pub async fn for_member(
        csrf_service: &CsrfService,
        current_user: &CurrentUser,
        session: &SessionInfo,
    ) -> Self {
        let csrf_token = csrf_service
            .generate_token(&session.session_id)
            .await
            .unwrap_or_default();
        let is_admin = current_user.member.is_admin;
        // Read the cached latest stable release (written by the daily
        // background task) and decide whether this admin is behind it.
        // Dismissal is enforced client-side from the `update_dismissed`
        // cookie (the server has no per-request cookie here), so we pass
        // `None` — see `banner_for` and `templates/layouts/base.html`.
        let cached = crate::service::update_check::cached_latest();
        let update_banner = crate::service::update_check::banner_for(
            is_admin,
            crate::version::release_tag(),
            cached.as_ref(),
            None,
        );
        let update_readme_url = if update_banner.is_some() {
            format!("{}#update", crate::version::docs_url("README.md"))
        } else {
            String::new()
        };
        Self {
            current_user: Some(UserInfo {
                id: current_user.member.id.to_string(),
                username: current_user.member.username.clone(),
                email: current_user.member.email.clone(),
            }),
            is_admin,
            csrf_token,
            version: crate::version::current(),
            release_url: crate::version::release_tag().map(crate::version::release_url),
            update_banner,
            update_readme_url,
        }
    }

    /// Pre-auth pages (login, setup, password reset). No session, no
    /// CSRF binding — the layout still renders the meta tag so HTMX
    /// won't crash, but the token is empty. Forms on these pages POST
    /// to CSRF-exempt endpoints (login, signup) or supply tokens
    /// out-of-band (password reset link).
    pub fn for_anon() -> Self {
        Self {
            version: crate::version::current(),
            release_url: crate::version::release_tag().map(crate::version::release_url),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone)]
pub struct UserInfo {
    pub id: String,
    pub username: String,
    pub email: String,
}

// Make askama templates work with axum
pub struct HtmlTemplate<T>(pub T);

impl<T> IntoResponse for HtmlTemplate<T>
where
    T: Template,
{
    fn into_response(self) -> Response {
        match self.0.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to render template: {}", err),
            )
                .into_response(),
        }
    }
}
