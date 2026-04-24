//! Admin UI for email configuration. Lives at /portal/admin/settings/email
//! with a dedicated form (rather than editing individual settings
//! through the generic settings page), plus a "Send test email" button
//! so admins can verify their SMTP setup without shelling into the
//! server.

use askama::Template;
use axum::{
    extract::State,
    response::{IntoResponse, Redirect, Response},
    Extension,
    Form,
};
use serde::Deserialize;

use crate::{
    api::{
        middleware::auth::{CurrentUser, SessionInfo},
        state::AppState,
    },
    email::{self, templates::{WelcomeHtml, WelcomeText}},
    service::settings_service::{DbEmailConfig, UpdateEmailConfig},
    web::templates::{HtmlTemplate, UserInfo},
};
use crate::web::portal::is_admin;

#[derive(Template)]
#[template(path = "admin/email_settings.html")]
pub struct AdminEmailSettingsTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub csrf_token: String,
    pub mode: String,
    pub from_address: String,
    pub from_name: String,
    pub smtp_host: String,
    pub smtp_port: String,
    pub smtp_username: String,
    /// Whether a password is currently set (we never display the
    /// plaintext — just "set" or "not set").
    pub smtp_password_set: bool,
    /// Last-test status: "never", "ok", or "failed".
    pub last_test_status: String,
    pub last_test_at: String,
    pub last_test_error: String,
    pub flash_success: Option<String>,
    pub flash_error: Option<String>,
}

pub async fn email_settings_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> Response {
    render_page(state, current_user, session_info, None, None).await
}

async fn render_page(
    state: AppState,
    current_user: CurrentUser,
    session_info: SessionInfo,
    flash_success: Option<String>,
    flash_error: Option<String>,
) -> Response {
    if !is_admin(&current_user.member) {
        return Redirect::to("/portal/dashboard").into_response();
    }

    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let csrf_token = state.service_context.csrf_service
        .generate_token(&session_info.session_id)
        .await
        .unwrap_or_else(|_| String::new());

    let cfg = state.service_context.settings_service
        .get_email_config()
        .await
        .unwrap_or_default();

    let last_test_at = state.service_context.settings_service
        .get_value("email.last_test_at").await.unwrap_or_default();
    let last_test_ok = state.service_context.settings_service
        .get_bool("email.last_test_ok").await.unwrap_or(false);
    let last_test_error = state.service_context.settings_service
        .get_value("email.last_test_error").await.unwrap_or_default();

    let last_test_status = if last_test_at.is_empty() {
        "never"
    } else if last_test_ok {
        "ok"
    } else {
        "failed"
    }.to_string();

    HtmlTemplate(AdminEmailSettingsTemplate {
        current_user: Some(user_info),
        is_admin: true,
        csrf_token,
        mode: cfg.mode,
        from_address: cfg.from_address,
        from_name: cfg.from_name,
        smtp_host: cfg.smtp_host,
        smtp_port: cfg.smtp_port.to_string(),
        smtp_username: cfg.smtp_username,
        smtp_password_set: !cfg.smtp_password.is_empty(),
        last_test_status,
        last_test_at,
        last_test_error,
        flash_success,
        flash_error,
    }).into_response()
}

#[derive(Debug, Deserialize)]
pub struct UpdateEmailForm {
    pub csrf_token: String,
    pub mode: String,
    pub from_address: String,
    pub from_name: String,
    pub smtp_host: String,
    pub smtp_port: String,
    pub smtp_username: String,
    /// Blank string means "leave existing password alone". A special
    /// sentinel "__CLEAR__" means "remove the stored password". Any
    /// other value replaces it.
    pub smtp_password: String,
}

pub async fn update_email_settings(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Form(form): Form<UpdateEmailForm>,
) -> Response {
    if !is_admin(&current_user.member) {
        return Redirect::to("/portal/dashboard").into_response();
    }

    // CSRF is also enforced by middleware, but double-check explicitly
    // so admins get a clear error if something went wrong.
    let csrf_valid = state.service_context.csrf_service
        .validate_token(&session_info.session_id, &form.csrf_token)
        .await
        .unwrap_or(false);
    if !csrf_valid {
        return render_page(
            state, current_user, session_info,
            None,
            Some("Invalid CSRF token. Please reload and try again.".to_string()),
        ).await;
    }

    // Validate inputs
    if form.mode != "log" && form.mode != "smtp" {
        return render_page(
            state, current_user, session_info,
            None,
            Some("Mode must be 'log' or 'smtp'.".to_string()),
        ).await;
    }

    let smtp_port: u16 = match form.smtp_port.parse() {
        Ok(p) if p > 0 => p,
        _ => {
            return render_page(
                state, current_user, session_info,
                None,
                Some("SMTP port must be a positive number (common values: 587, 465, 25).".to_string()),
            ).await;
        }
    };

    // Password field semantics:
    //   ""            -> keep the existing stored password
    //   "__CLEAR__"   -> clear the stored password
    //   anything else -> update with the new value
    let smtp_password = match form.smtp_password.as_str() {
        "" => None,
        "__CLEAR__" => Some(String::new()),
        other => Some(other.to_string()),
    };

    let update = UpdateEmailConfig {
        mode: form.mode,
        from_address: form.from_address,
        from_name: form.from_name,
        smtp_host: form.smtp_host,
        smtp_port,
        smtp_username: form.smtp_username,
        smtp_password,
    };

    match state.service_context.settings_service
        .update_email_config(update, current_user.member.id)
        .await
    {
        Ok(_) => render_page(
            state, current_user, session_info,
            Some("Email settings saved.".to_string()),
            None,
        ).await,
        Err(e) => {
            tracing::error!("update_email_config failed: {}", e);
            render_page(
                state, current_user, session_info,
                None,
                Some(format!("Failed to save settings: {}", e)),
            ).await
        }
    }
}

/// Send a test email to the logged-in admin's address using the
/// current (live, DB-sourced) configuration. Returns an HTMX-friendly
/// fragment that replaces the status area on the settings page.
pub async fn send_test_email(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {
    if !is_admin(&current_user.member) {
        return axum::response::Html(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded text-sm">Access denied</div>"#.to_string()
        );
    }

    let admin_email = current_user.member.email.clone();
    let full_name = current_user.member.full_name.clone();

    // Look up org name for the subject line / body.
    let org_name = state.service_context.settings_service
        .get_value("org.name").await
        .ok().filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Coterie".to_string());

    let portal_url = format!(
        "{}/portal/dashboard",
        state.settings.server.base_url.trim_end_matches('/'),
    );

    // Borrow the welcome template as a generic "friendly test" body —
    // keeps the template surface smaller. The admin sees it and knows
    // SMTP is working.
    let html = WelcomeHtml { full_name: &full_name, org_name: &org_name, portal_url: &portal_url };
    let text = WelcomeText { full_name: &full_name, org_name: &org_name, portal_url: &portal_url };
    let message = match email::message_from_templates(
        admin_email.clone(),
        format!("[Test] Email from {} is working", org_name),
        &html,
        &text,
    ) {
        Ok(m) => m,
        Err(e) => return test_result_html(false, &format!("Template error: {}", e)),
    };

    let (ok, error_text) = match state.service_context.email_sender.send(&message).await {
        Ok(()) => (true, String::new()),
        Err(e) => (false, e.to_string()),
    };

    // Record the test result so the settings page reflects it on next load.
    let _ = state.service_context.settings_service
        .record_email_test(ok, &error_text, current_user.member.id)
        .await;

    if ok {
        test_result_html(true, &format!("Test email sent to {}.", admin_email))
    } else {
        test_result_html(false, &error_text)
    }
}

fn test_result_html(ok: bool, detail: &str) -> axum::response::Html<String> {
    let escaped = crate::web::escape_html(detail);
    let (bg, fg, icon) = if ok {
        ("bg-green-50", "text-green-900",
         r#"<svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/></svg>"#)
    } else {
        ("bg-red-50", "text-red-900",
         r#"<svg class="h-5 w-5 inline" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/></svg>"#)
    };
    axum::response::Html(format!(
        r#"<div id="test-result" class="mt-2 p-3 {bg} {fg} rounded-md text-sm">{icon} {detail}</div>"#,
        bg = bg, fg = fg, icon = icon, detail = escaped,
    ))
}
