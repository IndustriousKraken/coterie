use std::sync::Arc;

use askama::Template;
use axum::{
    extract::State,
    response::{IntoResponse, Response},
    Extension,
};
use axum_extra::extract::CookieJar;
use serde::Deserialize;
use sqlx::SqlitePool;

use super::MemberInfo;
use crate::{
    api::middleware::auth::{CurrentUser, SessionInfo},
    auth::{AuthService, CsrfService},
    config::Settings,
    repository::MemberRepository,
    service::{audit_service::AuditService, membership_type_service::MembershipTypeService},
    web::templates::{filters, BaseContext, HtmlTemplate},
};

#[derive(Template)]
#[template(path = "portal/profile.html")]
pub struct ProfileTemplate {
    pub base: BaseContext,
    pub member: MemberInfo,
}

pub async fn profile_page(
    State(membership_type_service): State<Arc<MembershipTypeService>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> impl IntoResponse {
    let membership_type_name = membership_type_service
        .get(current_user.member.membership_type_id)
        .await
        .ok()
        .flatten()
        .map(|mt| mt.name)
        .unwrap_or_else(|| "(unknown)".to_string());

    let member_info = MemberInfo {
        id: current_user.member.id,
        username: current_user.member.username.clone(),
        full_name: current_user.member.full_name.clone(),
        email: current_user.member.email.clone(),
        status: current_user.member.status,
        membership_type: membership_type_name,
        joined_at: current_user.member.joined_at,
        dues_paid_until: current_user.member.dues_paid_until,
    };

    let template = ProfileTemplate {
        base: BaseContext::for_member(&csrf_service, &current_user, &session_info).await,
        member: member_info,
    };

    HtmlTemplate(template)
}

#[derive(Debug, Deserialize)]
pub struct UpdateProfileRequest {
    pub full_name: String,
    #[allow(dead_code)]
    pub csrf_token: String,
}

pub async fn update_profile(
    State(member_repo): State<Arc<dyn MemberRepository>>,
    Extension(current_user): Extension<CurrentUser>,
    axum::Form(form): axum::Form<UpdateProfileRequest>,
) -> axum::response::Response {
    use crate::domain::UpdateMemberRequest;

    let update = UpdateMemberRequest {
        full_name: Some(form.full_name.clone()),
        ..Default::default()
    };

    match member_repo.update(current_user.member.id, update).await {
        Ok(_) => {
            // Redirect back to profile with success message
            axum::response::Response::builder()
                .status(200)
                .header("HX-Redirect", "/portal/profile")
                .header(
                    "X-Toast",
                    r#"{"message":"Profile updated successfully!","type":"success"}"#,
                )
                .body(axum::body::Body::empty())
                .unwrap()
        }
        Err(e) => {
            let html = format!(
                "<div class=\"p-4 bg-red-50 text-red-800 rounded-md\">Failed to update profile: {}</div>",
                crate::web::escape_html(&e.to_string())
            );
            axum::response::Html(html).into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdatePasswordRequest {
    pub current_password: String,
    pub new_password: String,
    pub confirm_password: String,
    #[allow(dead_code)]
    pub csrf_token: String,
}

pub async fn update_password(
    State(db_pool): State<SqlitePool>,
    State(member_repo): State<Arc<dyn MemberRepository>>,
    State(auth_service): State<Arc<AuthService>>,
    State(settings): State<Arc<Settings>>,
    State(audit_service): State<Arc<AuditService>>,
    Extension(current_user): Extension<CurrentUser>,
    jar: CookieJar,
    axum::Form(form): axum::Form<UpdatePasswordRequest>,
) -> Response {
    // Validate passwords match
    if form.new_password != form.confirm_password {
        return axum::response::Html(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">
                New passwords do not match
            </div>"#
                .to_string(),
        )
        .into_response();
    }

    // Validate password complexity
    if let Err(msg) = crate::auth::validate_password(&form.new_password) {
        return axum::response::Html(format!(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">{}</div>"#,
            crate::web::escape_html(msg)
        ))
        .into_response();
    }

    // Verify current password
    let password_hash = crate::auth::get_password_hash(&db_pool, &current_user.member.email)
        .await
        .ok()
        .flatten();

    let password_valid = if let Some(hash) = password_hash {
        crate::auth::AuthService::verify_password(&form.current_password, &hash)
            .await
            .unwrap_or(false)
    } else {
        false
    };

    if !password_valid {
        return axum::response::Html(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">
                Current password is incorrect
            </div>"#
                .to_string(),
        )
        .into_response();
    }

    // Hash new password and update
    use argon2::password_hash::{rand_core::OsRng, SaltString};
    use argon2::{Argon2, PasswordHasher};

    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();

    let new_hash = match argon2.hash_password(form.new_password.as_bytes(), &salt) {
        Ok(hash) => hash.to_string(),
        Err(_) => {
            return axum::response::Html(
                r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">
                    Failed to update password
                </div>"#
                    .to_string(),
            )
            .into_response();
        }
    };

    // Update password in database
    if member_repo
        .update_password_hash(current_user.member.id, &new_hash)
        .await
        .is_err()
    {
        return axum::response::Html(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">
                Failed to update password
            </div>"#
                .to_string(),
        )
        .into_response();
    }

    // Kill every existing session for this member — including the
    // caller's current cookie. If a stolen-cookie scenario was the
    // reason for the password change, this is the action that closes
    // the gap. Mirrors `reset_password_handler` in
    // src/web/templates/reset.rs: log the failure loudly but still
    // report success, because the password DID change.
    if let Err(e) = auth_service
        .invalidate_all_sessions(current_user.member.id)
        .await
    {
        tracing::error!(
            "Password change for member {} succeeded but session invalidation FAILED — \
             stale sessions may still be valid: {}",
            current_user.member.id,
            e
        );
    }

    // Mint a fresh session for the caller so they aren't logged out on
    // the device they just changed their password from.
    let new_jar = match auth_service.create_session(current_user.member.id, 24).await {
        Ok((_session, token)) => {
            let cookie = auth_service
                .create_session_cookie(&token, settings.server.cookies_are_secure());
            jar.add(cookie)
        }
        Err(e) => {
            tracing::error!(
                "Password change for member {} succeeded but re-issuing the caller's \
                 session FAILED — caller will need to log in again: {}",
                current_user.member.id,
                e
            );
            jar
        }
    };

    audit_service
        .log(
            Some(current_user.member.id),
            "password_change",
            "member",
            &current_user.member.id.to_string(),
            None,
            Some("via portal"),
            None,
        )
        .await;

    (
        new_jar,
        axum::response::Html(
            r#"<div class="p-3 bg-green-50 text-green-800 rounded-md text-sm">
                Password updated successfully!
            </div>"#
                .to_string(),
        ),
    )
        .into_response()
}
