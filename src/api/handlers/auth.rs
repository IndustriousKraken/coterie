use std::sync::Arc;

use anyhow::Context;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::{
    api::state::{self, LoginLimiter},
    auth::{self, AuthService, PendingLoginService, TotpService},
    config::Settings,
    error::{AppError, Result},
    service::audit_service::AuditService,
};

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct TotpRequiredResponse {
    pub message: String,
    pub pending_token: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginTotpRequest {
    /// 6-digit TOTP code OR recovery code (XXXX-XXXX-XXXX).
    pub code: String,
    /// Optional override for the cookie-borne pending token. Non-cookie
    /// JSON clients (mobile apps, curl) send this in the body instead.
    pub pending_token: Option<String>,
}

pub async fn login(
    State(auth_service): State<Arc<AuthService>>,
    State(settings): State<Arc<Settings>>,
    State(login_limiter): State<LoginLimiter>,
    State(totp_service): State<Arc<TotpService>>,
    State(pending_login_service): State<Arc<PendingLoginService>>,
    State(db_pool): State<SqlitePool>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(req): Json<LoginRequest>,
) -> Result<Response> {
    // Rate-limit login attempts per IP
    let ip = state::client_ip(&headers, settings.server.trust_forwarded_for());
    if !login_limiter.0.check_and_record(ip) {
        return Err(AppError::TooManyRequests);
    }

    // Get password hash from database
    let password_hash = auth::get_password_hash(&db_pool, &req.email).await?;

    let password_hash = match password_hash {
        Some(h) => h,
        None => {
            // User not found — burn Argon2 time to prevent timing-based enumeration.
            auth::AuthService::verify_dummy(&req.password).await;
            return Err(AppError::Unauthorized);
        }
    };

    // Verify password
    if !auth::AuthService::verify_password(&req.password, &password_hash).await? {
        return Err(AppError::Unauthorized);
    }

    // Get member
    let member = auth::get_member_by_email(&db_pool, &req.email)
        .await?
        .ok_or(AppError::Unauthorized)?;

    // Reject login for Pending/Suspended. Expired is allowed through so
    // the member can reach the restoration flow and update payment.
    use crate::domain::MemberStatus;
    match member.status {
        MemberStatus::Active | MemberStatus::Honorary | MemberStatus::Expired => {}
        MemberStatus::Pending | MemberStatus::Suspended => {
            return Err(AppError::Forbidden);
        }
    }

    // 2FA branch: mirror the web handler. A correct password alone must
    // not be a usable login for a TOTP-enrolled member. We also skip the
    // session-invalidation sweep and defer it to /auth/login/totp — doing
    // it here would let an attacker who guessed the password log the
    // victim out at will, a DoS vector that 2FA otherwise prevents.
    // Fail closed: a transient failure of the enrollment query must NOT
    // skip the 2FA branch. Surface as 500 instead so an attacker can't
    // race a DB blip into a password-only session.
    let totp_enabled = totp_service
        .is_enabled(member.id)
        .await
        .context("failed to query TOTP enrollment status")
        .map_err(|e| AppError::Internal(format!("{e:#}")))?;
    if totp_enabled {
        let pending_token = pending_login_service.create(member.id, false).await?;
        let pending_cookie = crate::auth::pending_login::create_cookie(
            &pending_token,
            settings.server.cookies_are_secure(),
        );
        let jar = jar.add(pending_cookie);
        return Ok((
            StatusCode::ACCEPTED,
            jar,
            Json(TotpRequiredResponse {
                message: "2fa_required".to_string(),
                pending_token,
            }),
        )
            .into_response());
    }

    // Invalidate pre-existing sessions to prevent session fixation.
    let _ = auth_service.invalidate_all_sessions(member.id).await;

    // Create session (returns both session and token)
    let (_session, token) = auth_service.create_session(member.id, 24).await?;

    // Create cookie with the actual token. The Secure flag tracks whether
    // the deployment is TLS-terminated; see ServerConfig::cookies_are_secure.
    let cookie = auth_service.create_session_cookie(&token, settings.server.cookies_are_secure());

    Ok((
        jar.add(cookie),
        Json(LoginResponse {
            message: "Login successful".to_string(),
        }),
    )
        .into_response())
}

/// Second-factor endpoint for the JSON login flow. The client must
/// present the `pending_login` token that `/auth/login` minted —
/// either as the `pending_login` cookie or, for non-cookie JSON
/// callers, in the request body. On a valid TOTP code (or one-shot
/// recovery code) the handler consumes the pending row, performs the
/// session-fixation sweep deferred from `/auth/login`, and issues a
/// fresh session cookie.
#[allow(clippy::too_many_arguments)]
pub async fn login_totp(
    State(auth_service): State<Arc<AuthService>>,
    State(settings): State<Arc<Settings>>,
    State(login_limiter): State<LoginLimiter>,
    State(totp_service): State<Arc<TotpService>>,
    State(pending_login_service): State<Arc<PendingLoginService>>,
    State(db_pool): State<SqlitePool>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(req): Json<LoginTotpRequest>,
) -> Result<Response> {
    // Share the per-IP budget with /auth/login so an attacker holding a
    // stolen password can't switch surfaces to get a fresh allowance
    // when brute-forcing the 6-digit TOTP code.
    let ip = state::client_ip(&headers, settings.server.trust_forwarded_for());
    if !login_limiter.0.check_and_record(ip) {
        return Err(AppError::TooManyRequests);
    }

    let token = jar
        .get(crate::auth::pending_login::COOKIE_NAME)
        .map(|c| c.value().to_string())
        .or_else(|| req.pending_token.clone());

    let token = match token {
        Some(t) if !t.is_empty() => t,
        _ => {
            let jar = jar.add(crate::auth::pending_login::create_clear_cookie());
            return Ok((
                StatusCode::UNAUTHORIZED,
                jar,
                Json(serde_json::json!({
                    "error": "Unauthorized",
                })),
            )
                .into_response());
        }
    };

    let pending = match pending_login_service.find(&token).await? {
        Some(p) => p,
        None => {
            let jar = jar.add(crate::auth::pending_login::create_clear_cookie());
            return Ok((
                StatusCode::UNAUTHORIZED,
                jar,
                Json(serde_json::json!({
                    "error": "Unauthorized",
                })),
            )
                .into_response());
        }
    };

    let member = {
        use crate::repository::{MemberRepository, SqliteMemberRepository};
        let repo = SqliteMemberRepository::new(db_pool.clone());
        match repo.find_by_id(pending.member_id).await? {
            Some(m) => m,
            None => return Err(AppError::Unauthorized),
        }
    };

    // Try TOTP first; if that fails, try the recovery-code path. Their
    // formats don't overlap (6 digits vs hyphenated alphanumeric), so a
    // valid submission only ever satisfies one branch. On total failure
    // we deliberately leave the pending row in place so the client may
    // retry until expiry (the pending TTL is the per-attempt budget).
    let totp_ok = totp_service
        .verify_for_member(member.id, &req.code, &member.email)
        .await
        .unwrap_or(false);
    let used_recovery = if totp_ok {
        false
    } else {
        crate::auth::recovery_codes::try_consume(&db_pool, member.id, &req.code)
            .await
            .unwrap_or(false)
    };
    if !totp_ok && !used_recovery {
        return Err(AppError::Unauthorized);
    }

    // Atomically consume the pending row so a parallel retry can't issue
    // a second session. Belt-and-suspenders: also wipe any other pending
    // rows for this member.
    let consumed = pending_login_service.consume(&token).await?;
    if consumed.is_none() {
        // Lost a race with another window or with expiry — make the
        // client retry from /auth/login.
        let jar = jar.add(crate::auth::pending_login::create_clear_cookie());
        return Ok((
            StatusCode::UNAUTHORIZED,
            jar,
            Json(serde_json::json!({
                "error": "Unauthorized",
            })),
        )
            .into_response());
    }
    let _ = pending_login_service.delete_for_member(member.id).await;

    // Session-fixation sweep deferred from the password-only step.
    let _ = auth_service.invalidate_all_sessions(member.id).await;

    let (_session, session_token) = auth_service.create_session(member.id, 24).await?;

    let session_cookie =
        auth_service.create_session_cookie(&session_token, settings.server.cookies_are_secure());
    let clear_pending = crate::auth::pending_login::create_clear_cookie();

    if used_recovery {
        let remaining = crate::auth::recovery_codes::remaining_count(&db_pool, member.id)
            .await
            .unwrap_or(0);
        tracing::info!(
            "Member {} logged in via recovery code (JSON); {} codes remaining",
            member.id,
            remaining,
        );
    }

    let jar = jar.add(session_cookie).add(clear_pending);
    Ok((
        jar,
        Json(LoginResponse {
            message: "Login successful".to_string(),
        }),
    )
        .into_response())
}

/// CSRF is enforced at the application root by
/// `csrf_protect_unless_exempt`, so this handler runs only after a
/// valid token has been seen. We don't re-validate here.
pub async fn logout(
    State(auth_service): State<Arc<AuthService>>,
    State(csrf_service): State<Arc<auth::CsrfService>>,
    State(audit_service): State<Arc<AuditService>>,
    jar: CookieJar,
) -> Result<(CookieJar, StatusCode)> {
    if let Some(session_cookie) = jar.get("session") {
        if let Ok(Some(session)) = auth_service.validate_session(session_cookie.value()).await {
            let _ = csrf_service.delete_token(&session.id).await;
            audit_service
                .log(
                    Some(session.member_id),
                    "logout",
                    "session",
                    &session.id,
                    None,
                    None,
                    None,
                )
                .await;
        }
        let _ = auth_service
            .invalidate_session(session_cookie.value())
            .await;
    }

    let jar = jar.add(auth::AuthService::create_logout_cookie());

    Ok((jar, StatusCode::NO_CONTENT))
}
