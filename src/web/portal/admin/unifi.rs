//! Admin UI for UniFi Access configuration. Lives at
//! `/portal/admin/settings/unifi`, mirroring the Discord settings page: a
//! single form for everything, the password write-only (blank keeps the
//! stored encrypted value), and a "Test connection" button that
//! authenticates to the controller without persisting. Config is DB-backed
//! and read at operation time, so a save takes effect with no restart.

use std::sync::Arc;

use askama::Template;
use axum::{
    extract::State,
    response::{IntoResponse, Response},
    Extension, Form,
};
use serde::Deserialize;

use crate::{
    api::middleware::auth::{CurrentUser, SessionInfo},
    auth::CsrfService,
    integrations::unifi,
    service::{
        audit_service::AuditService,
        settings_service::{SettingsService, UpdateUnifiConfig},
    },
    web::{
        portal::admin::test_result::test_result_html,
        templates::{BaseContext, HtmlTemplate},
    },
};

#[derive(Template)]
#[template(path = "admin/unifi_settings.html")]
pub struct UnifiSettingsTemplate {
    pub base: BaseContext,
    pub enabled: bool,
    pub controller_url: String,
    pub username: String,
    pub site_id: String,
    /// True if a password is on file (we never display the plaintext).
    pub password_set: bool,
    /// True if the encrypted password can't decrypt (session_secret rotated).
    pub password_undecryptable: bool,
    /// Last-test status: "never", "ok", or "failed".
    pub last_test_status: String,
    pub last_test_at: String,
    pub last_test_error: String,
    pub flash_success: Option<String>,
    pub flash_error: Option<String>,
}

pub async fn unifi_settings_page(
    State(settings_service): State<Arc<SettingsService>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> Response {
    render_page(
        &settings_service,
        &csrf_service,
        &current_user,
        &session_info,
        None,
        None,
    )
    .await
}

async fn render_page(
    settings_service: &SettingsService,
    csrf_service: &CsrfService,
    current_user: &CurrentUser,
    session_info: &SessionInfo,
    flash_success: Option<String>,
    flash_error: Option<String>,
) -> Response {
    let base = BaseContext::for_member(csrf_service, current_user, session_info).await;

    let password_undecryptable = settings_service.unifi_password_undecryptable().await;

    let cfg = settings_service
        .get_unifi_config()
        .await
        .unwrap_or_default();

    let last_test_at = settings_service
        .get_value("unifi.last_test_at")
        .await
        .unwrap_or_default();
    let last_test_ok = settings_service
        .get_bool("unifi.last_test_ok")
        .await
        .unwrap_or(false);
    let last_test_error = settings_service
        .get_value("unifi.last_test_error")
        .await
        .unwrap_or_default();

    let last_test_status = if last_test_at.is_empty() {
        "never"
    } else if last_test_ok {
        "ok"
    } else {
        "failed"
    }
    .to_string();

    HtmlTemplate(UnifiSettingsTemplate {
        base,
        enabled: cfg.enabled,
        controller_url: cfg.controller_url,
        username: cfg.username,
        site_id: cfg.site_id,
        password_set: !cfg.password.is_empty(),
        password_undecryptable,
        last_test_status,
        last_test_at,
        last_test_error,
        flash_success,
        flash_error,
    })
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct UpdateUnifiForm {
    pub csrf_token: String,
    /// HTML checkbox: present when checked, absent otherwise.
    #[serde(default)]
    pub enabled: Option<String>,
    pub controller_url: String,
    pub username: String,
    pub site_id: String,
    /// Same convention as the Discord bot token: "" = leave alone,
    /// "__CLEAR__" = remove, anything else = update.
    pub password: String,
}

pub async fn update_unifi_settings(
    State(settings_service): State<Arc<SettingsService>>,
    State(csrf_service): State<Arc<CsrfService>>,
    State(audit_service): State<Arc<AuditService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Form(form): Form<UpdateUnifiForm>,
) -> Response {
    // Belt-and-suspenders CSRF (the middleware already validated).
    let csrf_valid = csrf_service
        .validate_token(&session_info.session_id, &form.csrf_token)
        .await
        .unwrap_or(false);
    if !csrf_valid {
        return render_page(
            &settings_service,
            &csrf_service,
            &current_user,
            &session_info,
            None,
            Some("Invalid CSRF token. Reload and try again.".to_string()),
        )
        .await;
    }

    // Controller URL: if non-empty, must be an http(s) URL.
    let controller_url = form.controller_url.trim().to_string();
    if !(controller_url.is_empty()
        || controller_url.starts_with("http://")
        || controller_url.starts_with("https://"))
    {
        return render_page(
            &settings_service,
            &csrf_service,
            &current_user,
            &session_info,
            None,
            Some("Controller URL should start with http:// or https://".to_string()),
        )
        .await;
    }

    let password = match form.password.as_str() {
        "" => None,
        "__CLEAR__" => Some(String::new()),
        other => Some(other.to_string()),
    };

    let update = UpdateUnifiConfig {
        enabled: form.enabled.is_some(),
        controller_url,
        username: form.username.trim().to_string(),
        site_id: form.site_id.trim().to_string(),
        password,
    };

    match settings_service
        .update_unifi_config(update, current_user.member.id)
        .await
    {
        Ok(_) => {
            // Audit but don't include the password in the row (it'd be
            // plaintext from the form — defeats the encryption-at-rest).
            audit_service
                .log(
                    Some(current_user.member.id),
                    "update_unifi_config",
                    "settings",
                    "unifi",
                    None,
                    None,
                    None,
                )
                .await;
            render_page(
                &settings_service,
                &csrf_service,
                &current_user,
                &session_info,
                Some("UniFi settings saved.".to_string()),
                None,
            )
            .await
        }
        Err(e) => {
            tracing::error!("update_unifi_config failed: {}", e);
            render_page(
                &settings_service,
                &csrf_service,
                &current_user,
                &session_info,
                None,
                Some(format!("Failed to save: {}", e)),
            )
            .await
        }
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct TestUnifiForm {
    /// The password typed into the form, if any. Blank → test the stored
    /// password. Never persisted by this handler.
    #[serde(default)]
    pub controller_url: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
}

/// Authenticate to the UniFi controller with the submitted (or stored)
/// credentials and report success/failure WITHOUT persisting them. Only the
/// test-result status (last_test_*) is recorded, never the credentials.
pub async fn test_unifi_connection(
    State(settings_service): State<Arc<SettingsService>>,
    Extension(current_user): Extension<CurrentUser>,
    Form(form): Form<TestUnifiForm>,
) -> impl IntoResponse {
    // Prefer submitted values; fall back to the stored (decrypted) config.
    let stored = settings_service.get_unifi_config().await;

    let controller_url = match form.controller_url.trim() {
        "" => stored
            .as_ref()
            .map(|c| c.controller_url.clone())
            .unwrap_or_default(),
        other => other.to_string(),
    };
    let username = match form.username.trim() {
        "" => stored
            .as_ref()
            .map(|c| c.username.clone())
            .unwrap_or_default(),
        other => other.to_string(),
    };
    let password = match form.password.trim() {
        "" | "__CLEAR__" => match &stored {
            Ok(c) => c.password.clone(),
            Err(e) => {
                return test_result_html(
                    "unifi-test-result",
                    false,
                    &format!("Couldn't load stored UniFi config: {}", e),
                );
            }
        },
        other => other.to_string(),
    };

    let (ok, detail) = unifi::test_connection(&controller_url, &username, &password).await;

    if let Err(e) = settings_service
        .record_unifi_test(ok, if ok { "" } else { &detail }, current_user.member.id)
        .await
    {
        tracing::warn!("UniFi test completed but result wasn't persisted: {}", e);
    }

    test_result_html("unifi-test-result", ok, &detail)
}
