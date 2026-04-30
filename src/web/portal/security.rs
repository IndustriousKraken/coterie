//! Member-facing 2FA (TOTP) management page. Available to every
//! logged-in member; admin promotion is independent of TOTP enrollment.
//!
//! The flow is deliberately split into HTMX fragments rather than full
//! page reloads — the page renders the QR + recovery codes once, into
//! a `<div id="totp-section">` that the buttons replace as they go.
//! Form posts return HTML fragments (not JSON) so HTMX can swap them
//! in directly.

use askama::Template;
use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse, Response},
    Extension,
};
use serde::Deserialize;

use crate::{
    api::{
        middleware::auth::{CurrentUser, SessionInfo},
        state::AppState,
    },
    web::templates::{HtmlTemplate, UserInfo},
};
use super::{MemberInfo, is_admin};

#[derive(Template)]
#[template(path = "portal/security.html")]
pub struct SecurityTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub member: MemberInfo,
    pub csrf_token: String,
    pub totp_enabled: bool,
    pub recovery_codes_remaining: usize,
    /// True when the admin-mandatory toggle is on AND this user is an
    /// unenrolled admin — drives the "you must enroll" banner. Page
    /// is reachable in this state because `/portal/profile/security`
    /// is in the active-only routes (not gated by admin enforcement).
    pub admin_must_enroll: bool,
}

#[derive(Debug, Deserialize)]
pub struct SecurityQuery {
    /// Set to "admin_totp_required" by `require_admin_redirect` when
    /// the toggle bounced an admin here. We surface a friendly banner
    /// on top of the standard buttons.
    pub reason: Option<String>,
}

#[derive(Template)]
#[template(path = "portal/security_enroll_qr.html")]
pub struct EnrollQrTemplate {
    pub csrf_token: String,
    pub secret_base32: String,
    pub qr_svg: String,
    pub error: Option<String>,
}

#[derive(Template)]
#[template(path = "portal/security_recovery_codes.html")]
pub struct RecoveryCodesTemplate {
    pub csrf_token: String,
    pub codes: Vec<String>,
    /// Headline copy varies between "you just enabled 2FA" and "you
    /// regenerated your codes". Caller passes the right one.
    pub heading: String,
    pub subheading: String,
}

pub async fn security_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Query(query): Query<SecurityQuery>,
) -> impl IntoResponse {
    let csrf_token = state.service_context.csrf_service
        .generate_token(&session_info.session_id)
        .await
        .unwrap_or_else(|_| String::new());

    let totp_enabled = state.service_context.totp_service
        .is_enabled(current_user.member.id)
        .await
        .unwrap_or(false);

    let remaining = if totp_enabled {
        crate::auth::recovery_codes::remaining_count(
            &state.service_context.db_pool,
            current_user.member.id,
        ).await.unwrap_or(0)
    } else { 0 };

    // Banner conditions: admin, no TOTP, and either the toggle is on
    // OR the user just got bounced here (?reason=admin_totp_required).
    // Showing the banner ALSO when the toggle is on (not only after a
    // bounce) means an admin who navigates here directly can see why
    // they're being asked to enroll.
    let enforce = state.service_context.settings_service
        .get_setting("auth.require_totp_for_admins").await
        .ok()
        .map(|s| s.value == "true")
        .unwrap_or(false);
    let bounced = query.reason.as_deref() == Some("admin_totp_required");
    let admin_must_enroll = is_admin(&current_user.member)
        && !totp_enabled
        && (enforce || bounced);

    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };
    let member_info = MemberInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        full_name: current_user.member.full_name.clone(),
        email: current_user.member.email.clone(),
        status: current_user.member.status.as_str().to_string(),
        membership_type: current_user.member.membership_type.as_str().to_string(),
        joined_at: current_user.member.joined_at.format("%B %d, %Y").to_string(),
        dues_paid_until: current_user.member.dues_paid_until
            .map(|d| d.format("%B %d, %Y").to_string()),
    };

    HtmlTemplate(SecurityTemplate {
        current_user: Some(user_info),
        is_admin: is_admin(&current_user.member),
        member: member_info,
        csrf_token,
        totp_enabled,
        recovery_codes_remaining: remaining,
        admin_must_enroll,
    })
}

// --------------------------------------------------------------------
// Enroll: POST /portal/profile/security/totp/enroll/start
// Returns the QR + a confirmation form. No persistence yet — the
// secret round-trips via a hidden field and is only saved once a
// valid code is entered.
// --------------------------------------------------------------------

pub async fn enroll_start(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> Response {
    // Refuse if already enrolled — nothing UX-coherent to do, and we'd
    // overwrite the existing secret. Re-enrollment goes via disable
    // → enroll deliberately.
    let already = state.service_context.totp_service
        .is_enabled(current_user.member.id).await.unwrap_or(false);
    if already {
        return Html(
            r#"<div class="p-3 bg-yellow-50 text-yellow-800 rounded-md text-sm">
                Two-factor authentication is already enabled.
            </div>"#.to_string()
        ).into_response();
    }

    let init = match state.service_context.totp_service
        .begin_enrollment(&current_user.member.email)
    {
        Ok(i) => i,
        Err(e) => {
            tracing::error!("begin_enrollment failed: {}", e);
            return Html(
                r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">
                    Couldn't start enrollment. Please try again.
                </div>"#.to_string()
            ).into_response();
        }
    };

    let csrf_token = state.service_context.csrf_service
        .generate_token(&session_info.session_id).await
        .unwrap_or_else(|_| String::new());

    HtmlTemplate(EnrollQrTemplate {
        csrf_token,
        secret_base32: init.secret_base32,
        qr_svg: init.qr_svg,
        error: None,
    }).into_response()
}

// --------------------------------------------------------------------
// Enroll confirm: POST /portal/profile/security/totp/enroll/confirm
// Verifies the code against the round-tripped secret, persists, and
// returns a fragment showing the freshly-generated recovery codes.
// --------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct EnrollConfirmRequest {
    pub secret_base32: String,
    pub code: String,
    #[allow(dead_code)]
    pub csrf_token: String,
}

pub async fn enroll_confirm(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    axum::Form(form): axum::Form<EnrollConfirmRequest>,
) -> Response {
    let ok = match state.service_context.totp_service
        .confirm_enrollment(
            current_user.member.id,
            &form.secret_base32,
            &form.code,
            &current_user.member.email,
        ).await
    {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("confirm_enrollment failed: {}", e);
            return error_html("Couldn't verify code. Please try again.");
        }
    };

    let csrf_token = state.service_context.csrf_service
        .generate_token(&session_info.session_id).await
        .unwrap_or_else(|_| String::new());

    if !ok {
        // Re-render the enroll form with an error so the user can retry
        // without losing the QR / typed-in secret.
        return HtmlTemplate(EnrollQrTemplate {
            csrf_token,
            secret_base32: form.secret_base32,
            qr_svg: String::new(), // QR isn't needed on retry — secret unchanged
            error: Some("Code didn't match — try the next one your app shows.".to_string()),
        }).into_response();
    }

    // Code accepted. Issue recovery codes (this is the ONLY time the
    // user sees them) and render the codes-display fragment.
    let codes = match crate::auth::recovery_codes::issue_for_member(
        &state.service_context.db_pool,
        current_user.member.id,
    ).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("issue_for_member failed: {}", e);
            // Roll back enrollment so we don't leave the member in a
            // half-state with no recovery codes — they'd be locked out
            // if their authenticator app got wiped.
            let _ = state.service_context.totp_service
                .disable(current_user.member.id).await;
            return error_html("Couldn't finalize 2FA setup. Please try again.");
        }
    };

    state.service_context.audit_service.log(
        Some(current_user.member.id),
        "totp_enroll",
        "member",
        &current_user.member.id.to_string(),
        None, None, None,
    ).await;

    HtmlTemplate(RecoveryCodesTemplate {
        csrf_token,
        codes,
        heading: "Two-factor authentication enabled".to_string(),
        subheading:
            "Save these recovery codes somewhere safe. Each works exactly once \
             if you ever lose access to your authenticator app. Coterie can't \
             show them again.".to_string(),
    }).into_response()
}

// --------------------------------------------------------------------
// Disable: POST /portal/profile/security/totp/disable
// Requires a current TOTP code (or recovery code). On success, clears
// the secret + recovery codes + any pending_login rows.
// --------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct DisableRequest {
    pub code: String,
    #[allow(dead_code)]
    pub csrf_token: String,
}

pub async fn disable(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    axum::Form(form): axum::Form<DisableRequest>,
) -> Response {
    let totp_ok = state.service_context.totp_service
        .verify_for_member(current_user.member.id, &form.code, &current_user.member.email)
        .await.unwrap_or(false);
    let recovery_ok = if totp_ok {
        false
    } else {
        crate::auth::recovery_codes::try_consume(
            &state.service_context.db_pool,
            current_user.member.id,
            &form.code,
        ).await.unwrap_or(false)
    };
    if !totp_ok && !recovery_ok {
        return error_html("Code didn't match. 2FA is still enabled.");
    }

    if let Err(e) = state.service_context.totp_service.disable(current_user.member.id).await {
        tracing::error!("totp disable failed: {}", e);
        return error_html("Couldn't disable 2FA. Please try again.");
    }

    state.service_context.audit_service.log(
        Some(current_user.member.id),
        "totp_disable",
        "member",
        &current_user.member.id.to_string(),
        None, None, None,
    ).await;

    // Tell HTMX to reload the page so the buttons / status flip back.
    let mut headers = axum::http::HeaderMap::new();
    headers.insert("HX-Refresh", "true".parse().unwrap());
    (axum::http::StatusCode::OK, headers, "").into_response()
}

// --------------------------------------------------------------------
// Regenerate recovery codes: POST /portal/profile/security/totp/recovery-codes/regenerate
// Same authentication as disable. Wipes + reissues. Old codes are
// useless from this point on.
// --------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct RegenerateRequest {
    pub code: String,
    #[allow(dead_code)]
    pub csrf_token: String,
}

pub async fn regenerate_recovery_codes(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    axum::Form(form): axum::Form<RegenerateRequest>,
) -> Response {
    // Must already be enrolled — regenerate is meaningless otherwise.
    let enabled = state.service_context.totp_service
        .is_enabled(current_user.member.id).await.unwrap_or(false);
    if !enabled {
        return error_html("Two-factor authentication isn't enabled.");
    }

    let totp_ok = state.service_context.totp_service
        .verify_for_member(current_user.member.id, &form.code, &current_user.member.email)
        .await.unwrap_or(false);
    let recovery_ok = if totp_ok {
        false
    } else {
        crate::auth::recovery_codes::try_consume(
            &state.service_context.db_pool,
            current_user.member.id,
            &form.code,
        ).await.unwrap_or(false)
    };
    if !totp_ok && !recovery_ok {
        return error_html("Code didn't match. Recovery codes were not regenerated.");
    }

    let codes = match crate::auth::recovery_codes::issue_for_member(
        &state.service_context.db_pool,
        current_user.member.id,
    ).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("issue_for_member (regenerate) failed: {}", e);
            return error_html("Couldn't generate new codes. Please try again.");
        }
    };

    state.service_context.audit_service.log(
        Some(current_user.member.id),
        "totp_recovery_regenerate",
        "member",
        &current_user.member.id.to_string(),
        None, None, None,
    ).await;

    let csrf_token = state.service_context.csrf_service
        .generate_token(&session_info.session_id).await
        .unwrap_or_else(|_| String::new());
    HtmlTemplate(RecoveryCodesTemplate {
        csrf_token,
        codes,
        heading: "New recovery codes generated".to_string(),
        subheading:
            "Your old codes no longer work. Save these somewhere safe — \
             each works exactly once.".to_string(),
    }).into_response()
}

fn error_html(msg: &str) -> Response {
    Html(format!(
        r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">{}</div>"#,
        crate::web::escape_html(msg),
    )).into_response()
}
