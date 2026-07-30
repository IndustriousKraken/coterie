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
    /// Member-editable org-defined fields with this member's values;
    /// the card is hidden when none exist.
    pub custom_fields: Vec<crate::web::portal::admin::member_fields::FieldRow>,
}

pub async fn profile_page(
    State(membership_type_service): State<Arc<MembershipTypeService>>,
    State(member_field_service): State<
        Arc<crate::service::member_field_service::MemberFieldService>,
    >,
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

    let custom_fields = member_field_service
        .fields_for(
            current_user.member.id,
            crate::service::member_field_service::FieldScope::Member,
        )
        .await
        .map(|f| crate::web::portal::admin::member_fields::field_rows_with_values(&f))
        .unwrap_or_default();

    let template = ProfileTemplate {
        base: BaseContext::for_member(&csrf_service, &current_user, &session_info).await,
        member: member_info,
        custom_fields,
    };

    HtmlTemplate(template)
}

#[derive(Debug, Deserialize)]
pub struct UpdateProfileRequest {
    pub full_name: String,
    #[allow(dead_code)]
    pub csrf_token: String,
}

/// Member-side door to `members.full_name`. The other door is the
/// unauthenticated `POST /public/signup` (`src/api/handlers/public.rs`);
/// both write the same column, so the bound here intentionally mirrors
/// signup's 200-character cap and trimmed-value semantics. Change one,
/// change the other — otherwise the two doors drift and the looser one
/// becomes the storage-abuse vector.
pub async fn update_profile(
    State(member_repo): State<Arc<dyn MemberRepository>>,
    Extension(current_user): Extension<CurrentUser>,
    axum::Form(form): axum::Form<UpdateProfileRequest>,
) -> axum::response::Response {
    use crate::domain::UpdateMemberRequest;

    let inline_error = |msg: &str| -> axum::response::Response {
        axum::response::Html(format!(
            "<div class=\"p-4 bg-red-50 text-red-800 rounded-md\">{}</div>",
            crate::web::escape_html(msg)
        ))
        .into_response()
    };

    let full_name = form.full_name.trim();
    if full_name.is_empty() {
        return inline_error("Name is required");
    }
    if full_name.chars().count() > 200 {
        return inline_error("Name is too long (max 200 characters)");
    }

    let update = UpdateMemberRequest {
        full_name: Some(full_name.to_string()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        routing::post,
        Router,
    };
    use tower::ServiceExt;

    use crate::{
        domain::{CreateMemberRequest, Member},
        repository::SqliteMemberRepository,
    };

    async fn member_and_app() -> (Member, Arc<dyn MemberRepository>, Router) {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();

        let member_repo: Arc<dyn MemberRepository> = Arc::new(SqliteMemberRepository::new(pool));
        let member = member_repo
            .create(CreateMemberRequest {
                email: "member@example.com".to_string(),
                username: "member".to_string(),
                full_name: "Original Name".to_string(),
                password: "p4ssword_long_enough".to_string(),
                membership_type_id: None,
                ..Default::default()
            })
            .await
            .unwrap();

        let app = Router::new()
            .route("/portal/profile", post(update_profile))
            .layer(Extension(CurrentUser {
                member: member.clone(),
            }))
            .with_state(member_repo.clone());

        (member, member_repo, app)
    }

    async fn post_full_name(app: Router, full_name: &str) -> (StatusCode, String, Option<String>) {
        let body =
            serde_urlencoded::to_string([("full_name", full_name), ("csrf_token", "t")]).unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/portal/profile")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        let status = resp.status();
        let redirect = resp
            .headers()
            .get("HX-Redirect")
            .map(|v| v.to_str().unwrap().to_string());
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8(body.to_vec()).unwrap(), redirect)
    }

    #[tokio::test]
    async fn profile_update_rejects_overlong_full_name() {
        let (member, member_repo, app) = member_and_app().await;

        let (_, body, _) = post_full_name(app, &"a".repeat(201)).await;
        assert!(
            body.contains("too long"),
            "expected the inline too-long error fragment, got: {body}"
        );

        let stored = member_repo.find_by_id(member.id).await.unwrap().unwrap();
        assert_eq!(stored.full_name, "Original Name");
    }

    #[tokio::test]
    async fn profile_update_rejects_blank_full_name() {
        let (member, member_repo, app) = member_and_app().await;

        let (_, body, _) = post_full_name(app, "   ").await;
        assert!(
            body.contains("Name is required"),
            "expected the inline required error fragment, got: {body}"
        );

        let stored = member_repo.find_by_id(member.id).await.unwrap().unwrap();
        assert_eq!(stored.full_name, "Original Name");
    }

    #[tokio::test]
    async fn profile_update_trims_and_persists_valid_name() {
        let (member, member_repo, app) = member_and_app().await;

        let (status, _, redirect) = post_full_name(app, "  Ada Lovelace  ").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(redirect.as_deref(), Some("/portal/profile"));

        let stored = member_repo.find_by_id(member.id).await.unwrap().unwrap();
        assert_eq!(stored.full_name, "Ada Lovelace");
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
    headers: axum::http::HeaderMap,
    jar: CookieJar,
    axum::Form(form): axum::Form<UpdatePasswordRequest>,
) -> Response {
    let ip = crate::api::state::client_ip(&headers, settings.server.trust_forwarded_for());
    let member_id = current_user.member.id;

    // Validate passwords match
    if form.new_password != form.confirm_password {
        crate::util::auth_log::denied(
            "auth.password_changed",
            "password_mismatch",
            Some(member_id),
            Some(ip),
            None,
            None,
        );
        return axum::response::Html(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">
                New passwords do not match
            </div>"#
                .to_string(),
        )
        .into_response();
    }

    // Validate password complexity
    if let Err(rule) =
        crate::auth::validate_password_logged(&form.new_password, Some(member_id), Some(ip))
    {
        return axum::response::Html(format!(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">{}</div>"#,
            crate::web::escape_html(rule.message())
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
        crate::util::auth_log::denied(
            "auth.password_changed",
            "bad_password",
            Some(member_id),
            Some(ip),
            None,
            Some("current_password"),
        );
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
    let new_jar = match auth_service
        .create_session(current_user.member.id, 24)
        .await
    {
        Ok((_session, token)) => {
            let cookie =
                auth_service.create_session_cookie(&token, settings.server.cookies_are_secure());
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
            Some(member_id),
            "password_change",
            "member",
            &member_id.to_string(),
            None,
            Some("via portal"),
            Some(&ip.to_string()),
        )
        .await;
    crate::util::auth_log::ok(
        "auth.password_changed",
        Some(member_id),
        Some(ip),
        None,
        None,
    );

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

/// Member save of their own member-editable custom fields. Locked
/// (non-member-editable) fields are rejected in the service even under
/// a crafted POST.
pub async fn update_custom_fields(
    State(member_field_service): State<
        Arc<crate::service::member_field_service::MemberFieldService>,
    >,
    Extension(current_user): Extension<CurrentUser>,
    axum::Form(form): axum::Form<Vec<(String, String)>>,
) -> axum::response::Response {
    use crate::service::member_field_service::FieldScope;
    use crate::web::portal::admin::member_fields::field_pairs;

    let pairs = field_pairs(&form);
    match member_field_service
        .save_values(
            current_user.member.id,
            current_user.member.id,
            &pairs,
            FieldScope::Member,
        )
        .await
    {
        Ok(()) => axum::response::Response::builder()
            .status(200)
            .header("HX-Redirect", "/portal/profile")
            .header(
                "X-Toast",
                r#"{"message":"Details saved!","type":"success"}"#,
            )
            .body(axum::body::Body::empty())
            .unwrap(),
        Err(e) => {
            let html = format!(
                "<div class=\"p-4 bg-red-50 text-red-800 rounded-md\">Save failed: {}</div>",
                crate::web::escape_html(&e.to_string())
            );
            axum::response::Html(html).into_response()
        }
    }
}
