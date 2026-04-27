use askama::Template;
use axum::{
    extract::{State, Query},
    response::{IntoResponse, Response, Redirect},
    http::{StatusCode, HeaderMap, header},
    Json,
};
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};
use crate::{
    api::state::AppState,
    web::templates::HtmlTemplate,
};

#[derive(Template)]
#[template(path = "auth/login.html")]
pub struct LoginTemplate {
    pub current_user: Option<super::UserInfo>,
    pub is_admin: bool,
    pub redirect_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginQuery {
    pub redirect: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    pub remember_me: Option<bool>,
    pub redirect_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub success: bool,
    pub redirect: Option<String>,
    pub error: Option<String>,
}

// GET /login - redirect to dashboard if already logged in
pub async fn login_page(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(query): Query<LoginQuery>,
) -> Response {
    // Check if user already has a valid session
    if let Some(session_cookie) = jar.get("session") {
        if let Ok(Some(session)) = state.service_context.auth_service
            .validate_session(session_cookie.value())
            .await
        {
            // Already logged in — send Expired members to the restoration
            // page directly, everyone else to the dashboard.
            use crate::domain::MemberStatus;
            let dest = match state.service_context.member_repo
                .find_by_id(session.member_id)
                .await
                .ok()
                .flatten()
                .map(|m| m.status)
            {
                Some(MemberStatus::Expired) => "/portal/restore",
                _ => "/portal/dashboard",
            };
            return Redirect::to(dest).into_response();
        }
    }

    let template = LoginTemplate {
        current_user: None,
        is_admin: false,
        redirect_url: query.redirect,
    };
    HtmlTemplate(template).into_response()
}

// POST /auth/login
pub async fn login_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(credentials): Json<LoginRequest>,
) -> Response {
    // Rate-limit login attempts per IP
    let ip = crate::api::state::client_ip(
        &headers,
        state.settings.server.trust_forwarded_for(),
    );
    if !state.login_limiter.check_and_record(ip) {
        return (StatusCode::TOO_MANY_REQUESTS, Json(LoginResponse {
            success: false,
            redirect: None,
            error: Some("Too many login attempts. Please try again later.".to_string()),
        })).into_response();
    }

    // Find member by username or email
    let member = state.service_context.member_repo
        .find_by_username(&credentials.username)
        .await
        .ok()
        .flatten();
    
    let member = if member.is_none() {
        state.service_context.member_repo
            .find_by_email(&credentials.username)
            .await
            .ok()
            .flatten()
    } else {
        member
    };
    
    if let Some(member) = member {
        // Get password hash from database
        let password_hash = crate::auth::get_password_hash(
            &state.service_context.db_pool,
            &member.email
        ).await.ok().flatten();

        // Verify password
        let password_valid = if let Some(hash) = password_hash {
            crate::auth::AuthService::verify_password(
                &credentials.password,
                &hash
            ).await.unwrap_or(false)
        } else {
            false
        };

        if password_valid {
            // Reject login for Pending/Suspended — they shouldn't have a
            // portal session at all. Expired members are allowed in so they
            // can reach the restoration flow and update payment.
            use crate::domain::MemberStatus;
            match member.status {
                MemberStatus::Active | MemberStatus::Honorary | MemberStatus::Expired => {}
                MemberStatus::Pending => {
                    return (StatusCode::FORBIDDEN, Json(LoginResponse {
                        success: false,
                        redirect: None,
                        error: Some("Your account is awaiting admin approval.".to_string()),
                    })).into_response();
                }
                MemberStatus::Suspended => {
                    return (StatusCode::FORBIDDEN, Json(LoginResponse {
                        success: false,
                        redirect: None,
                        error: Some("Your account has been suspended. Please contact an administrator.".to_string()),
                    })).into_response();
                }
            }

            // Invalidate any pre-existing sessions for this member before
            // creating the new one. Prevents session fixation: if an attacker
            // planted a cookie in the victim's browser, that token is now
            // dead.
            let _ = state.service_context.auth_service
                .invalidate_all_sessions(member.id)
                .await;

            // Create session
            let (_session, token) = state.service_context.auth_service
                .create_session(
                    member.id,
                    if credentials.remember_me.unwrap_or(false) { 24 * 30 } else { 24 }
                )
                .await
                .unwrap();
            // Create session cookie. Secure flag is driven by server config
            // so local http dev still works while TLS deployments get it set.
            let max_age_secs = if credentials.remember_me.unwrap_or(false) {
                60 * 60 * 24 * 30 // 30 days
            } else {
                60 * 60 * 24 // 24 hours
            };
            let secure_attr = if state.settings.server.cookies_are_secure() {
                "; Secure"
            } else {
                ""
            };
            let cookie_value = format!(
                "session={}; HttpOnly; SameSite=Lax; Path=/; Max-Age={}{}",
                token, max_age_secs, secure_attr,
            );

            // Expired members go straight to the restoration flow. Active/
            // Honorary go to the originally-requested URL (if validated) or
            // the dashboard. Path validation guards against open-redirect.
            let default_destination = if member.status == MemberStatus::Expired {
                "/portal/restore".to_string()
            } else {
                "/portal/dashboard".to_string()
            };
            let redirect_url = credentials.redirect_url
                .filter(|url| url.starts_with("/portal/") && !url.contains(".."))
                .unwrap_or(default_destination);

            // Build response headers. Both values are server-controlled
            // (token is hex; redirect_url has been path-validated to
            // /portal/...), but using parse().unwrap() panics on any
            // future-proofing surprise. Treat parse failures as a 500
            // rather than crashing the request handler.
            let cookie_header = match cookie_value.parse() {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!("Failed to construct session cookie header: {}", e);
                    return (StatusCode::INTERNAL_SERVER_ERROR, Json(LoginResponse {
                        success: false, redirect: None,
                        error: Some("Login failed. Please try again.".to_string()),
                    })).into_response();
                }
            };
            let redirect_header = match redirect_url.parse() {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!("Invalid redirect URL after login (will use dashboard): {}", e);
                    "/portal/dashboard".parse().expect("static path always parses")
                }
            };

            let mut headers = HeaderMap::new();
            headers.insert(header::SET_COOKIE, cookie_header);
            headers.insert("HX-Redirect", redirect_header);

            return (StatusCode::OK, headers, Json(LoginResponse {
                success: true,
                redirect: Some(redirect_url),
                error: None,
            })).into_response();
        }
    } else {
        // User not found — run Argon2 against a dummy hash so the response
        // latency is indistinguishable from a wrong-password attempt.
        crate::auth::AuthService::verify_dummy(&credentials.password).await;
    }

    // Invalid credentials
    (StatusCode::UNAUTHORIZED, Json(LoginResponse {
        success: false,
        redirect: None,
        error: Some("Invalid username or password".to_string()),
    })).into_response()
}

// POST /logout
//
// CSRF: SameSite=Lax cookies still ride along on top-level POST
// navigations, so a cross-origin attacker page could `<form action=
// "https://coterie.example/logout" method="POST">…</form>` and force-
// log out a victim. Annoying rather than dangerous, but it lets an
// attacker push the victim into a re-auth screen they could phish.
// We require the X-CSRF-Token header (HTMX stamps it from the meta
// tag) — direct-form-POST CSRF therefore fails fast.
pub async fn logout_handler(
    State(state): State<AppState>,
    headers_in: HeaderMap,
    jar: CookieJar,
) -> impl IntoResponse {
    // Properly invalidate session and CSRF token
    if let Some(session_cookie) = jar.get("session") {
        // Get session to find its ID for CSRF token deletion
        if let Ok(Some(session)) = state.service_context.auth_service
            .validate_session(session_cookie.value())
            .await
        {
            // Verify the CSRF token before doing anything destructive.
            let token = headers_in
                .get("X-CSRF-Token")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            let csrf_ok = state.service_context.csrf_service
                .validate_token(&session.id, token)
                .await
                .unwrap_or(false);
            if !csrf_ok {
                let mut headers = HeaderMap::new();
                headers.insert("HX-Redirect", "/login".parse().unwrap());
                return (StatusCode::FORBIDDEN, headers);
            }
            // Delete CSRF token for this session
            let _ = state.service_context.csrf_service
                .delete_token(&session.id)
                .await;
            // Audit-trail the session lifecycle. Login is logged in
            // the audit_service; logout was silent until this entry.
            state.service_context.audit_service.log(
                Some(session.member_id),
                "logout",
                "session",
                &session.id,
                None,
                None,
                None,
            ).await;
        }
        // Invalidate the session
        let _ = state.service_context.auth_service
            .invalidate_session(session_cookie.value())
            .await;
    }

    let mut headers = HeaderMap::new();
    headers.insert(
        header::SET_COOKIE,
        "session=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0".parse().unwrap()
    );
    headers.insert("HX-Redirect", "/login".parse().unwrap());

    (StatusCode::OK, headers)
}