//! Admin UI for Discord integration. Mirrors the email settings
//! layout: a single form for everything plus a "Test connection"
//! button that hits Discord's API and shows the bot's identity (or
//! the exact error) inline.

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
    integrations::{discord::DiscordIntegration, discord_client::DiscordClient},
    service::settings_service::UpdateDiscordConfig,
    web::templates::{HtmlTemplate, UserInfo},
};

#[derive(Template)]
#[template(path = "admin/discord_settings.html")]
pub struct DiscordSettingsTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub csrf_token: String,
    pub enabled: bool,
    pub guild_id: String,
    pub member_role_id: String,
    pub expired_role_id: String,
    pub events_channel_id: String,
    pub announcements_channel_id: String,
    pub admin_alerts_channel_id: String,
    pub invite_url: String,
    /// True if a token is on file (we never display the plaintext).
    pub bot_token_set: bool,
    /// True if the encrypted token can't decrypt (session_secret rotated).
    pub token_undecryptable: bool,
    /// Last-test status: "never", "ok", or "failed".
    pub last_test_status: String,
    pub last_test_at: String,
    pub last_test_error: String,
    pub flash_success: Option<String>,
    pub flash_error: Option<String>,
}

pub async fn discord_settings_page(
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
    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let csrf_token = state.service_context.csrf_service
        .generate_token(&session_info.session_id)
        .await
        .unwrap_or_default();

    let token_undecryptable = state.service_context.settings_service
        .discord_token_undecryptable().await;

    let cfg = state.service_context.settings_service
        .get_discord_config().await
        .unwrap_or_default();

    let last_test_at = state.service_context.settings_service
        .get_value("discord.last_test_at").await.unwrap_or_default();
    let last_test_ok = state.service_context.settings_service
        .get_bool("discord.last_test_ok").await.unwrap_or(false);
    let last_test_error = state.service_context.settings_service
        .get_value("discord.last_test_error").await.unwrap_or_default();

    let last_test_status = if last_test_at.is_empty() {
        "never"
    } else if last_test_ok {
        "ok"
    } else {
        "failed"
    }.to_string();

    HtmlTemplate(DiscordSettingsTemplate {
        current_user: Some(user_info),
        is_admin: true,
        csrf_token,
        enabled: cfg.enabled,
        guild_id: cfg.guild_id,
        member_role_id: cfg.member_role_id,
        expired_role_id: cfg.expired_role_id,
        events_channel_id: cfg.events_channel_id,
        announcements_channel_id: cfg.announcements_channel_id,
        admin_alerts_channel_id: cfg.admin_alerts_channel_id,
        invite_url: cfg.invite_url,
        bot_token_set: !cfg.bot_token.is_empty(),
        token_undecryptable,
        last_test_status,
        last_test_at,
        last_test_error,
        flash_success,
        flash_error,
    }).into_response()
}

#[derive(Debug, Deserialize)]
pub struct UpdateDiscordForm {
    pub csrf_token: String,
    /// HTML checkbox: present when checked, absent otherwise.
    #[serde(default)]
    pub enabled: Option<String>,
    pub guild_id: String,
    pub member_role_id: String,
    pub expired_role_id: String,
    pub events_channel_id: String,
    pub announcements_channel_id: String,
    pub admin_alerts_channel_id: String,
    pub invite_url: String,
    /// Same convention as SMTP password: "" = leave alone,
    /// "__CLEAR__" = remove, anything else = update.
    pub bot_token: String,
}

pub async fn update_discord_settings(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Form(form): Form<UpdateDiscordForm>,
) -> Response {
    // Belt-and-suspenders CSRF (the middleware already validated).
    let csrf_valid = state.service_context.csrf_service
        .validate_token(&session_info.session_id, &form.csrf_token)
        .await
        .unwrap_or(false);
    if !csrf_valid {
        return render_page(
            state, current_user, session_info,
            None,
            Some("Invalid CSRF token. Reload and try again.".to_string()),
        ).await;
    }

    // Validate any provided snowflakes (empty is OK — means "not configured").
    if let Some(err) = first_invalid_snowflake(&[
        ("Guild ID", &form.guild_id),
        ("Member role ID", &form.member_role_id),
        ("Expired role ID", &form.expired_role_id),
        ("Events channel ID", &form.events_channel_id),
        ("Announcements channel ID", &form.announcements_channel_id),
        ("Admin alerts channel ID", &form.admin_alerts_channel_id),
    ]) {
        return render_page(state, current_user, session_info, None, Some(err)).await;
    }

    // Invite URL: if non-empty, must look like https://discord.gg/... or .com.
    if !form.invite_url.is_empty()
        && !(form.invite_url.starts_with("https://discord.gg/")
            || form.invite_url.starts_with("https://discord.com/invite/"))
    {
        return render_page(
            state, current_user, session_info,
            None,
            Some("Invite URL should start with https://discord.gg/ or https://discord.com/invite/".to_string()),
        ).await;
    }

    let bot_token = match form.bot_token.as_str() {
        "" => None,
        "__CLEAR__" => Some(String::new()),
        other => Some(other.to_string()),
    };

    let update = UpdateDiscordConfig {
        enabled: form.enabled.is_some(),
        guild_id: form.guild_id,
        member_role_id: form.member_role_id,
        expired_role_id: form.expired_role_id,
        events_channel_id: form.events_channel_id,
        announcements_channel_id: form.announcements_channel_id,
        admin_alerts_channel_id: form.admin_alerts_channel_id,
        invite_url: form.invite_url,
        bot_token,
    };

    match state.service_context.settings_service
        .update_discord_config(update, current_user.member.id)
        .await
    {
        Ok(_) => {
            // Audit but don't include bot token in the row (it'd be
            // plaintext from the form — defeats the encryption-at-rest).
            state.service_context.audit_service.log(
                Some(current_user.member.id),
                "update_discord_config",
                "settings",
                "discord",
                None, None, None,
            ).await;
            render_page(state, current_user, session_info,
                Some("Discord settings saved.".to_string()), None).await
        }
        Err(e) => {
            tracing::error!("update_discord_config failed: {}", e);
            render_page(state, current_user, session_info,
                None, Some(format!("Failed to save: {}", e))).await
        }
    }
}

/// Return the first input that's set but doesn't look like a valid
/// snowflake. Empty values are allowed (means "not configured").
fn first_invalid_snowflake(inputs: &[(&str, &str)]) -> Option<String> {
    for (label, val) in inputs {
        if !val.is_empty() && !crate::integrations::discord::is_valid_snowflake(val) {
            return Some(format!(
                "{} doesn't look like a valid Discord snowflake (17–20 digits). Got: {}",
                label, val
            ));
        }
    }
    None
}

/// Hit Discord's API with the current bot token and report what the
/// connection looks like. Used by the "Test connection" button.
pub async fn test_discord_connection(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {

    let cfg = match state.service_context.settings_service.get_discord_config().await {
        Ok(c) => c,
        Err(e) => {
            // Most likely the token can't be decrypted (session_secret rotated).
            return test_result_html(false, &format!("Couldn't load Discord config: {}", e));
        }
    };

    if cfg.bot_token.is_empty() {
        return test_result_html(false, "No bot token configured. Paste one above and save first.");
    }

    let client = DiscordClient::new(cfg.bot_token);
    let (ok, detail) = match client.get_current_user().await {
        Ok(user) => {
            let identity = match user.discriminator.as_deref() {
                Some(d) if !d.is_empty() && d != "0" => format!("{}#{}", user.username, d),
                _ => user.username,
            };
            (true, format!("Connected as Discord bot: {} (id: {})", identity, user.id))
        }
        Err(e) => (false, e.to_string()),
    };

    if let Err(e) = state.service_context.settings_service
        .record_discord_test(ok, if ok { "" } else { &detail }, current_user.member.id)
        .await
    {
        tracing::warn!("Discord test completed but result wasn't persisted: {}", e);
    }

    test_result_html(ok, &detail)
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
        r#"<div id="discord-test-result" class="mt-2 p-3 {bg} {fg} rounded-md text-sm">{icon} {detail}</div>"#,
        bg = bg, fg = fg, icon = icon, detail = escaped,
    ))
}

/// Reconcile every member's Discord roles against their current
/// status. Fired by the admin "Re-sync all roles" button. The same
/// logic also runs daily in a background task — this is the manual
/// trigger for "I just fixed a Discord outage, get state right NOW."
///
/// Returns an HTMX-friendly fragment that replaces the test-result
/// area with a summary.
pub async fn reconcile_roles(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
) -> impl IntoResponse {

    let cfg = match state.service_context.settings_service.get_discord_config().await {
        Ok(c) => c,
        Err(e) => return test_result_html(false, &format!("Couldn't load Discord config: {}", e)),
    };
    if !cfg.enabled || cfg.bot_token.is_empty() || cfg.guild_id.is_empty() {
        return test_result_html(false, "Discord integration isn't enabled or configured.");
    }

    let integration = DiscordIntegration::new(
        state.service_context.settings_service.clone(),
        state.settings.server.base_url.clone(),
    );
    let summary = integration
        .reconcile_all(state.service_context.member_repo.clone())
        .await;

    state.service_context.audit_service.log(
        Some(current_user.member.id),
        "discord_reconcile_manual",
        "settings",
        "discord",
        None, None, None,
    ).await;

    let detail = format!(
        "Reconciled {} member(s). Skipped {} with invalid Discord ID, {} pending.",
        summary.processed, summary.skipped_invalid_id, summary.skipped_pending,
    );
    test_result_html(true, &detail)
}
