//! Admin UI for Stripe configuration. Lives at
//! `/portal/admin/settings/stripe`, mirroring the Discord/email settings
//! pages: a single form for everything, secrets write-only (blank keeps
//! the stored encrypted value), a "Test connection" button that pings
//! the Stripe API without persisting, and a save that hot-reloads the
//! running client + webhook signing secret so changes take effect with
//! no restart.

use std::sync::Arc;
use std::time::Duration;

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
    domain::BillingMode,
    payments::StripeHandle,
    repository::MemberRepository,
    service::{
        audit_service::AuditService,
        settings_service::{SettingsService, UpdateStripeConfig},
    },
    web::{
        portal::admin::test_result::test_result_html,
        templates::{BaseContext, HtmlTemplate},
    },
};

#[derive(Template)]
#[template(path = "admin/stripe_settings.html")]
pub struct StripeSettingsTemplate {
    pub base: BaseContext,
    pub enabled: bool,
    pub publishable_key: String,
    pub success_url: String,
    pub cancel_url: String,
    /// True if a secret key is on file (we never display the plaintext).
    pub secret_key_set: bool,
    /// True if a webhook signing secret is on file.
    pub webhook_secret_set: bool,
    /// True if a stored secret can't be decrypted (session_secret rotated).
    pub secret_undecryptable: bool,
    /// Last-test status: "never", "ok", or "failed".
    pub last_test_status: String,
    pub last_test_at: String,
    pub last_test_error: String,
    /// When set, a disable was blocked pending confirmation: this many
    /// members are on `stripe_subscription`. The template renders the
    /// confirmation checkbox and a warning naming the count.
    pub pending_disable_count: Option<i64>,
    pub flash_success: Option<String>,
    pub flash_error: Option<String>,
}

pub async fn stripe_settings_page(
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
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn render_page(
    settings_service: &SettingsService,
    csrf_service: &CsrfService,
    current_user: &CurrentUser,
    session_info: &SessionInfo,
    pending_disable_count: Option<i64>,
    flash_success: Option<String>,
    flash_error: Option<String>,
) -> Response {
    let base = BaseContext::for_member(csrf_service, current_user, session_info).await;

    let secret_undecryptable = settings_service.stripe_secret_undecryptable().await;

    let cfg = settings_service
        .get_stripe_config()
        .await
        .unwrap_or_default();

    let last_test_at = settings_service
        .get_value("stripe.last_test_at")
        .await
        .unwrap_or_default();
    let last_test_ok = settings_service
        .get_bool("stripe.last_test_ok")
        .await
        .unwrap_or(false);
    let last_test_error = settings_service
        .get_value("stripe.last_test_error")
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

    HtmlTemplate(StripeSettingsTemplate {
        base,
        enabled: cfg.enabled,
        publishable_key: cfg.publishable_key,
        success_url: cfg.success_url,
        cancel_url: cfg.cancel_url,
        secret_key_set: !cfg.secret_key.is_empty(),
        webhook_secret_set: !cfg.webhook_secret.is_empty(),
        secret_undecryptable,
        last_test_status,
        last_test_at,
        last_test_error,
        pending_disable_count,
        flash_success,
        flash_error,
    })
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct UpdateStripeForm {
    pub csrf_token: String,
    /// HTML checkbox: present when checked, absent otherwise.
    #[serde(default)]
    pub enabled: Option<String>,
    pub publishable_key: String,
    pub success_url: String,
    pub cancel_url: String,
    /// "" = keep stored, "__CLEAR__" = remove, anything else = replace.
    pub secret_key: String,
    /// Same convention as `secret_key`, for the webhook signing secret.
    pub webhook_secret: String,
    /// Present (checked) when the admin has acknowledged disabling
    /// Stripe with live subscriptions — see the disable-safety guard.
    #[serde(default)]
    pub confirm_disable: Option<String>,
}

// Axum handler: every parameter is an extractor, so the arg count is
// inherent (state + CSRF + audit + member repo + stripe handle + user +
// session + form).
#[allow(clippy::too_many_arguments)]
pub async fn update_stripe_settings(
    State(settings_service): State<Arc<SettingsService>>,
    State(csrf_service): State<Arc<CsrfService>>,
    State(audit_service): State<Arc<AuditService>>,
    State(member_repo): State<Arc<dyn MemberRepository>>,
    State(stripe_handle): State<Arc<StripeHandle>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Form(form): Form<UpdateStripeForm>,
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
            None,
            Some("Invalid CSRF token. Reload and try again.".to_string()),
        )
        .await;
    }

    let enabled = form.enabled.is_some();
    let confirmed = form.confirm_disable.is_some();

    // Disable safety: turning Stripe OFF while members are on
    // `stripe_subscription` stops the webhook from crediting their
    // renewals. Require an explicit confirmation naming the count first.
    let currently_enabled = settings_service
        .get_stripe_config()
        .await
        .map(|c| c.enabled)
        .unwrap_or(false);
    if currently_enabled && !enabled && !confirmed {
        let affected = member_repo
            .count_by_billing_mode(BillingMode::StripeSubscription)
            .await
            .unwrap_or(0);
        if affected > 0 {
            return render_page(
                &settings_service,
                &csrf_service,
                &current_user,
                &session_info,
                Some(affected),
                None,
                Some(format!(
                    "{} member(s) are on Stripe subscriptions. Disabling Stripe stops \
                     the webhook from crediting their renewals. Check the confirmation \
                     box below and save again to proceed.",
                    affected
                )),
            )
            .await;
        }
    }

    let secret_key = match form.secret_key.as_str() {
        "" => None,
        "__CLEAR__" => Some(String::new()),
        other => Some(other.to_string()),
    };
    let webhook_secret = match form.webhook_secret.as_str() {
        "" => None,
        "__CLEAR__" => Some(String::new()),
        other => Some(other.to_string()),
    };

    let update = UpdateStripeConfig {
        enabled,
        publishable_key: form.publishable_key,
        success_url: form.success_url,
        cancel_url: form.cancel_url,
        secret_key,
        webhook_secret,
    };

    match settings_service
        .update_stripe_config(update, current_user.member.id)
        .await
    {
        Ok(_) => {
            // Hot-reload: rebuild the client + webhook signing secret so
            // charges and webhook verification pick up the new values
            // with no restart.
            stripe_handle.rebuild().await;

            // Audit but never record the keys (they'd be plaintext from
            // the form — defeats the encryption-at-rest).
            audit_service
                .log(
                    Some(current_user.member.id),
                    "update_stripe_config",
                    "settings",
                    "stripe",
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
                None,
                Some("Stripe settings saved.".to_string()),
                None,
            )
            .await
        }
        Err(e) => {
            tracing::error!("update_stripe_config failed: {}", e);
            render_page(
                &settings_service,
                &csrf_service,
                &current_user,
                &session_info,
                None,
                None,
                Some(format!("Failed to save: {}", e)),
            )
            .await
        }
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct TestStripeForm {
    /// The secret key typed into the form, if any. Blank → test the
    /// stored key. Never persisted by this handler.
    #[serde(default)]
    pub secret_key: String,
}

/// Validate a Stripe secret key against the live API without persisting
/// anything. Uses the submitted key if present, otherwise the stored
/// one. A successful `GET /v1/balance` proves the key is valid.
pub async fn test_stripe_connection(
    State(settings_service): State<Arc<SettingsService>>,
    Extension(current_user): Extension<CurrentUser>,
    Form(form): Form<TestStripeForm>,
) -> impl IntoResponse {
    // Prefer the submitted key; fall back to the stored (decrypted) one.
    let secret_key = match form.secret_key.trim() {
        "" | "__CLEAR__" => match settings_service.get_stripe_config().await {
            Ok(cfg) => cfg.secret_key,
            Err(e) => {
                return test_result_html(
                    "stripe-test-result",
                    false,
                    &format!("Couldn't load stored Stripe config: {}", e),
                );
            }
        },
        other => other.to_string(),
    };

    if secret_key.trim().is_empty() {
        return test_result_html(
            "stripe-test-result",
            false,
            "No secret key to test. Paste one above (or save one first).",
        );
    }

    let client = stripe::Client::new(secret_key);
    // GET /v1/balance needs no ID and works for any account — the
    // canonical "is this key valid?" ping. Wrap in a timeout so a hung
    // connection can't tie up the handler (async-stripe 0.39 has no
    // per-request timeout).
    let (ok, detail) = match tokio::time::timeout(
        Duration::from_secs(15),
        stripe::Balance::retrieve(&client, None),
    )
    .await
    {
        Ok(Ok(_)) => (
            true,
            "Connected to Stripe — the secret key is valid.".to_string(),
        ),
        Ok(Err(e)) => (false, format!("Stripe rejected the key: {}", e)),
        Err(_) => (false, "Stripe API timed out after 15s.".to_string()),
    };

    if let Err(e) = settings_service
        .record_stripe_test(ok, if ok { "" } else { &detail }, current_user.member.id)
        .await
    {
        tracing::warn!("Stripe test completed but result wasn't persisted: {}", e);
    }

    test_result_html("stripe-test-result", ok, &detail)
}
