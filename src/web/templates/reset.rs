//! Password reset flow:
//!   GET /forgot-password  -> form asking for email
//!   POST /forgot-password -> generate token + send email (always
//!                            returns the same response regardless of
//!                            whether the email matches a member, to
//!                            avoid enumeration)
//!   GET /reset-password?token=X  -> new-password form
//!   POST /reset-password  -> verify token, hash new password, update,
//!                            invalidate all sessions
//!
//! A refused reset renders the same body it always has but returns a
//! non-200 status, so "password changed" and "password refused" are
//! distinguishable in the app, proxy, and uptime logs. The body stays
//! deliberately vague; only the status got honest.

use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Form,
};
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::{
    api::state::RecoveryLimiter,
    auth::{self, AuthService},
    config::Settings,
    email::{
        self,
        templates::{ResetHtml, ResetText},
        EmailSender,
    },
    repository::MemberRepository,
    service::{audit_service::AuditService, settings_service::SettingsService},
    util::auth_log,
    web::templates::{BaseContext, HtmlTemplate},
};

// ----- Forgot password -----

#[derive(Template)]
#[template(path = "auth/forgot_password.html")]
pub struct ForgotPasswordTemplate {
    pub base: BaseContext,
    pub submitted: bool,
}

#[derive(Debug, Deserialize)]
pub struct ForgotPasswordForm {
    pub email: String,
}

pub async fn forgot_password_page() -> Response {
    HtmlTemplate(ForgotPasswordTemplate {
        base: BaseContext::for_anon(),
        submitted: false,
    })
    .into_response()
}

pub async fn forgot_password_handler(
    State(settings): State<Arc<Settings>>,
    State(recovery_limiter): State<RecoveryLimiter>,
    State(member_repo): State<Arc<dyn MemberRepository>>,
    State(db_pool): State<SqlitePool>,
    State(settings_service): State<Arc<SettingsService>>,
    State(email_sender): State<Arc<dyn EmailSender>>,
    headers: HeaderMap,
    Form(form): Form<ForgotPasswordForm>,
) -> Response {
    // Rate-limit so the endpoint can't be used as a mass-email
    // generator or to probe for valid addresses. On `recovery_limiter`,
    // NOT `login_limiter`: failed logins must not close the recovery
    // path for the member who just forgot their password.
    let ip = crate::api::state::client_ip(&headers, settings.server.trust_forwarded_for());
    if !recovery_limiter
        .0
        .check_and_record(ip, "auth.password_reset")
    {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            "Too many requests. Please try again later.",
        )
            .into_response();
    }

    let identifier = auth_log::safe_identifier(&form.email);

    // Look up the member. Whether or not we find one, return the same
    // response — leaking membership via this endpoint would undo the
    // enumeration protection we added on signup.
    if let Ok(Some(member)) = member_repo.find_by_email(&form.email).await {
        // Generate token and send email. Soft-fail: we don't expose any
        // error to the caller; the tracing log captures the failure.
        match auth::email_tokens::create_password_reset_token(
            &db_pool,
            member.id,
            chrono::Duration::hours(1),
        )
        .await
        {
            Ok(created) => {
                let reset_url = format!(
                    "{}/reset-password?token={}",
                    settings.server.base_url.trim_end_matches('/'),
                    created.token,
                );
                let org_name = settings_service
                    .get_value("org.name")
                    .await
                    .ok()
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "Coterie".to_string());
                let html = ResetHtml {
                    full_name: &member.full_name,
                    org_name: &org_name,
                    reset_url: &reset_url,
                };
                let text = ResetText {
                    full_name: &member.full_name,
                    org_name: &org_name,
                    reset_url: &reset_url,
                };
                match email::message_from_templates(
                    member.email.clone(),
                    format!("Reset your {} password", org_name),
                    &html,
                    &text,
                ) {
                    Ok(message) => {
                        if let Err(e) = email_sender.send(&message).await {
                            tracing::error!("Reset email send failed: {}", e);
                            auth_log::denied(
                                "auth.password_reset_requested",
                                "email_send_failed",
                                Some(member.id),
                                Some(ip),
                                Some(identifier),
                                None,
                            );
                        } else {
                            auth_log::ok(
                                "auth.password_reset_requested",
                                Some(member.id),
                                Some(ip),
                                Some(identifier),
                                None,
                            );
                        }
                    }
                    Err(e) => {
                        tracing::error!("Reset email render failed: {}", e);
                        auth_log::denied(
                            "auth.password_reset_requested",
                            "email_render_failed",
                            Some(member.id),
                            Some(ip),
                            Some(identifier),
                            None,
                        );
                    }
                }
            }
            Err(e) => {
                tracing::error!("Reset token create failed: {}", e);
                auth_log::denied(
                    "auth.password_reset_requested",
                    "token_create_failed",
                    Some(member.id),
                    Some(ip),
                    Some(identifier),
                    None,
                );
            }
        }
    } else {
        // Not a member (or DB error). The response is identical either
        // way — only the log knows the difference, which is the whole
        // reason this event exists.
        auth_log::denied(
            "auth.password_reset_requested",
            "unknown_email",
            None,
            Some(ip),
            Some(identifier),
            None,
        );
    }

    HtmlTemplate(ForgotPasswordTemplate {
        base: BaseContext::for_anon(),
        submitted: true,
    })
    .into_response()
}

// ----- Reset password -----

#[derive(Template)]
#[template(path = "auth/reset_password.html")]
pub struct ResetPasswordTemplate {
    pub base: BaseContext,
    pub token: String,
    pub error: Option<String>,
}

#[derive(Template)]
#[template(path = "auth/reset_password_result.html")]
pub struct ResetPasswordResultTemplate {
    pub base: BaseContext,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct ResetPasswordQuery {
    pub token: String,
}

#[derive(Debug, Deserialize)]
pub struct ResetPasswordForm {
    pub token: String,
    pub new_password: String,
    pub confirm_password: String,
}

pub async fn reset_password_page(Query(query): Query<ResetPasswordQuery>) -> Response {
    HtmlTemplate(ResetPasswordTemplate {
        base: BaseContext::for_anon(),
        token: query.token,
        error: None,
    })
    .into_response()
}

pub async fn reset_password_handler(
    State(db_pool): State<SqlitePool>,
    State(member_repo): State<Arc<dyn MemberRepository>>,
    State(auth_service): State<Arc<AuthService>>,
    State(audit_service): State<Arc<AuditService>>,
    State(settings): State<Arc<Settings>>,
    headers: HeaderMap,
    Form(form): Form<ResetPasswordForm>,
) -> Response {
    let ip = crate::api::state::client_ip(&headers, settings.server.trust_forwarded_for());

    // Client-side validation first (gives the form back with an error
    // message, without burning the one-shot token).
    if form.new_password != form.confirm_password {
        auth_log::denied(
            "auth.password_reset_completed",
            "password_mismatch",
            None,
            Some(ip),
            None,
            None,
        );
        return (
            StatusCode::BAD_REQUEST,
            HtmlTemplate(ResetPasswordTemplate {
                base: BaseContext::for_anon(),
                token: form.token,
                error: Some("Passwords do not match.".to_string()),
            }),
        )
            .into_response();
    }
    if let Err(rule) = crate::auth::validate_password_logged(&form.new_password, None, Some(ip)) {
        return (
            StatusCode::BAD_REQUEST,
            HtmlTemplate(ResetPasswordTemplate {
                base: BaseContext::for_anon(),
                token: form.token,
                error: Some(rule.message()),
            }),
        )
            .into_response();
    }

    // Consume the token atomically. Any further attempts with the same
    // token will return None.
    let consumed = match auth::email_tokens::consume_password_reset_token(&db_pool, &form.token)
        .await
    {
        Ok(Some(c)) => c,
        Ok(None) => {
            // The response can't say which of these it was; the log
            // must, or "the reset link didn't work" stays unanswerable.
            let denial =
                auth::email_tokens::classify_password_reset_failure(&db_pool, &form.token).await;
            auth_log::denied(
                "auth.password_reset_completed",
                denial.slug(),
                None,
                Some(ip),
                None,
                None,
            );
            return (
                StatusCode::BAD_REQUEST,
                HtmlTemplate(ResetPasswordResultTemplate {
                    base: BaseContext::for_anon(),
                    success: false,
                    message:
                        "This reset link is invalid or has expired. Request a new one and try again."
                            .to_string(),
                }),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!("Reset token consume failed: {}", e);
            auth_log::denied(
                "auth.password_reset_completed",
                "token_lookup_failed",
                None,
                Some(ip),
                None,
                None,
            );
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                HtmlTemplate(ResetPasswordResultTemplate {
                    base: BaseContext::for_anon(),
                    success: false,
                    message: "Something went wrong. Please try again.".to_string(),
                }),
            )
                .into_response();
        }
    };

    // Hash the new password and persist it.
    let new_hash = match AuthService::hash_password(&form.new_password).await {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("Password hash failed during reset: {}", e);
            auth_log::denied(
                "auth.password_reset_completed",
                "hash_failed",
                Some(consumed.member_id),
                Some(ip),
                None,
                None,
            );
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                HtmlTemplate(ResetPasswordResultTemplate {
                    base: BaseContext::for_anon(),
                    success: false,
                    message: "Something went wrong. Please try again.".to_string(),
                }),
            )
                .into_response();
        }
    };

    if let Err(e) = member_repo
        .update_password_hash(consumed.member_id, &new_hash)
        .await
    {
        tracing::error!("Password update failed: {}", e);
        auth_log::denied(
            "auth.password_reset_completed",
            "update_failed",
            Some(consumed.member_id),
            Some(ip),
            None,
            None,
        );
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            HtmlTemplate(ResetPasswordResultTemplate {
                base: BaseContext::for_anon(),
                success: false,
                message: "Something went wrong. Please try again.".to_string(),
            }),
        )
            .into_response();
    }

    // Invalidate all existing sessions — whoever had them might be
    // the compromised party. Also invalidate any other outstanding
    // reset tokens for this member. If either fails we still report
    // success (the password DID change), but we log loudly because a
    // failure here means the suspected attacker's session might
    // remain valid until natural expiry.
    if let Err(e) = auth_service
        .invalidate_all_sessions(consumed.member_id)
        .await
    {
        tracing::error!(
            "Password reset for member {} succeeded but session invalidation FAILED — \
             stale sessions may still be valid: {}",
            consumed.member_id,
            e
        );
    }
    if let Err(e) = auth::email_tokens::invalidate_password_reset_tokens_for_member(
        &db_pool,
        consumed.member_id,
    )
    .await
    {
        tracing::warn!(
            "Couldn't invalidate other reset tokens for member {}: {}",
            consumed.member_id,
            e
        );
    }

    // A completed reset is reviewable account history, not just runtime
    // telemetry — so it gets an audit row as well as the log event. The
    // row carries no part of the new credential.
    audit_service
        .log(
            Some(consumed.member_id),
            "password_reset",
            "member",
            &consumed.member_id.to_string(),
            None,
            Some("via reset link"),
            Some(&ip.to_string()),
        )
        .await;
    auth_log::ok(
        "auth.password_reset_completed",
        Some(consumed.member_id),
        Some(ip),
        None,
        None,
    );

    HtmlTemplate(ResetPasswordResultTemplate {
        base: BaseContext::for_anon(),
        success: true,
        message: "Your password has been reset. You can now log in with your new password."
            .to_string(),
    })
    .into_response()
}
