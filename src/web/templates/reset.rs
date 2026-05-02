//! Password reset flow:
//!   GET /forgot-password  -> form asking for email
//!   POST /forgot-password -> generate token + send email (always
//!                            returns the same response regardless of
//!                            whether the email matches a member, to
//!                            avoid enumeration)
//!   GET /reset-password?token=X  -> new-password form
//!   POST /reset-password  -> verify token, hash new password, update,
//!                            invalidate all sessions

use askama::Template;
use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Form,
};
use serde::Deserialize;

use crate::{
    api::state::AppState,
    auth::{AuthService, EmailTokenService},
    email::{self, templates::{ResetHtml, ResetText}},
    repository::MemberRepository,
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

pub async fn forgot_password_page(
    State(_state): State<AppState>,
) -> Response {
    HtmlTemplate(ForgotPasswordTemplate {
        base: BaseContext::for_anon(),
        submitted: false,
    }).into_response()
}

pub async fn forgot_password_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<ForgotPasswordForm>,
) -> Response {
    // Rate-limit so the endpoint can't be used as a mass-email
    // generator or to probe for valid addresses.
    let ip = crate::api::state::client_ip(&headers, state.settings.server.trust_forwarded_for());
    if !state.login_limiter.check_and_record(ip) {
        return (StatusCode::TOO_MANY_REQUESTS,
            "Too many requests. Please try again later."
        ).into_response();
    }

    // Look up the member. Whether or not we find one, return the same
    // response — leaking membership via this endpoint would undo the
    // enumeration protection we added on signup.
    if let Ok(Some(member)) = state.service_context.member_repo
        .find_by_email(&form.email).await
    {
        // Generate token and send email. Soft-fail: we don't expose any
        // error to the caller; the tracing log captures the failure.
        let service = EmailTokenService::password_reset(state.service_context.db_pool.clone());
        match service.create(member.id, chrono::Duration::hours(1)).await {
            Ok(created) => {
                let reset_url = format!(
                    "{}/reset-password?token={}",
                    state.settings.server.base_url.trim_end_matches('/'),
                    created.token,
                );
                let org_name = state.service_context.settings_service
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
                        if let Err(e) = state.service_context.email_sender.send(&message).await {
                            tracing::error!("Reset email send failed: {}", e);
                        }
                    }
                    Err(e) => tracing::error!("Reset email render failed: {}", e),
                }
            }
            Err(e) => tracing::error!("Reset token create failed: {}", e),
        }
    } else {
        // Not a member (or DB error). Don't tell the user either way.
        tracing::debug!("Forgot-password request for unknown email");
    }

    HtmlTemplate(ForgotPasswordTemplate {
        base: BaseContext::for_anon(),
        submitted: true,
    }).into_response()
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

pub async fn reset_password_page(
    State(_state): State<AppState>,
    Query(query): Query<ResetPasswordQuery>,
) -> Response {
    HtmlTemplate(ResetPasswordTemplate {
        base: BaseContext::for_anon(),
        token: query.token,
        error: None,
    }).into_response()
}

pub async fn reset_password_handler(
    State(state): State<AppState>,
    Form(form): Form<ResetPasswordForm>,
) -> Response {
    // Client-side validation first (gives the form back with an error
    // message, without burning the one-shot token).
    if form.new_password != form.confirm_password {
        return HtmlTemplate(ResetPasswordTemplate {
            base: BaseContext::for_anon(),
            token: form.token,
            error: Some("Passwords do not match.".to_string()),
        }).into_response();
    }
    if let Err(msg) = crate::auth::validate_password(&form.new_password) {
        return HtmlTemplate(ResetPasswordTemplate {
            base: BaseContext::for_anon(),
            token: form.token,
            error: Some(msg.to_string()),
        }).into_response();
    }

    // Consume the token atomically. Any further attempts with the same
    // token will return None.
    let service = EmailTokenService::password_reset(state.service_context.db_pool.clone());
    let consumed = match service.consume(&form.token).await {
        Ok(Some(c)) => c,
        Ok(None) => {
            return HtmlTemplate(ResetPasswordResultTemplate {
                base: BaseContext::for_anon(),
                success: false,
                message: "This reset link is invalid or has expired. Request a new one and try again.".to_string(),
            }).into_response();
        }
        Err(e) => {
            tracing::error!("Reset token consume failed: {}", e);
            return HtmlTemplate(ResetPasswordResultTemplate {
                base: BaseContext::for_anon(),
                success: false,
                message: "Something went wrong. Please try again.".to_string(),
            }).into_response();
        }
    };

    // Hash the new password and persist it.
    let new_hash = match AuthService::hash_password(&form.new_password).await {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("Password hash failed during reset: {}", e);
            return HtmlTemplate(ResetPasswordResultTemplate {
                base: BaseContext::for_anon(),
                success: false,
                message: "Something went wrong. Please try again.".to_string(),
            }).into_response();
        }
    };

    if let Err(e) = state.service_context.member_repo
        .update_password_hash(consumed.member_id, &new_hash).await
    {
        tracing::error!("Password update failed: {}", e);
        return HtmlTemplate(ResetPasswordResultTemplate {
            base: BaseContext::for_anon(),
            success: false,
            message: "Something went wrong. Please try again.".to_string(),
        }).into_response();
    }

    // Invalidate all existing sessions — whoever had them might be
    // the compromised party. Also invalidate any other outstanding
    // reset tokens for this member. If either fails we still report
    // success (the password DID change), but we log loudly because a
    // failure here means the suspected attacker's session might
    // remain valid until natural expiry.
    if let Err(e) = state.service_context.auth_service
        .invalidate_all_sessions(consumed.member_id).await
    {
        tracing::error!(
            "Password reset for member {} succeeded but session invalidation FAILED — \
             stale sessions may still be valid: {}",
            consumed.member_id, e
        );
    }
    if let Err(e) = service.invalidate_for_member(consumed.member_id).await {
        tracing::warn!(
            "Couldn't invalidate other reset tokens for member {}: {}",
            consumed.member_id, e
        );
    }

    HtmlTemplate(ResetPasswordResultTemplate {
        base: BaseContext::for_anon(),
        success: true,
        message: "Your password has been reset. You can now log in with your new password.".to_string(),
    }).into_response()
}
