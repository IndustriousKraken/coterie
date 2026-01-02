use askama::Template;
use axum::{
    extract::State,
    response::IntoResponse,
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
#[template(path = "portal/profile.html")]
pub struct ProfileTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub member: MemberInfo,
    pub csrf_token: String,
}

pub async fn profile_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> impl IntoResponse {
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
        status: format!("{:?}", current_user.member.status),
        membership_type: format!("{:?}", current_user.member.membership_type),
        joined_at: current_user.member.joined_at.format("%B %d, %Y").to_string(),
        dues_paid_until: current_user.member.dues_paid_until
            .map(|d| d.format("%B %d, %Y").to_string()),
    };

    // Generate CSRF token for this session
    let csrf_token = state.service_context.csrf_service
        .generate_token(&session_info.session_id)
        .await
        .unwrap_or_else(|_| "error".to_string());

    let template = ProfileTemplate {
        current_user: Some(user_info),
        is_admin: is_admin(&current_user.member),
        member: member_info,
        csrf_token,
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
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    axum::Form(form): axum::Form<UpdateProfileRequest>,
) -> axum::response::Response {
    use crate::domain::UpdateMemberRequest;

    let update = UpdateMemberRequest {
        full_name: Some(form.full_name.clone()),
        ..Default::default()
    };

    match state.service_context.member_repo.update(current_user.member.id, update).await {
        Ok(_) => {
            // Redirect back to profile with success message
            axum::response::Response::builder()
                .status(200)
                .header("HX-Redirect", "/portal/profile")
                .header("X-Toast", r#"{"message":"Profile updated successfully!","type":"success"}"#)
                .body(axum::body::Body::empty())
                .unwrap()
        }
        Err(e) => {
            let html = format!(
                "<div class=\"p-4 bg-red-50 text-red-800 rounded-md\">Failed to update profile: {}</div>",
                e
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
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    axum::Form(form): axum::Form<UpdatePasswordRequest>,
) -> impl IntoResponse {
    // Validate passwords match
    if form.new_password != form.confirm_password {
        return axum::response::Html(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">
                New passwords do not match
            </div>"#.to_string()
        );
    }

    // Validate password length
    if form.new_password.len() < 8 {
        return axum::response::Html(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">
                Password must be at least 8 characters
            </div>"#.to_string()
        );
    }

    // Verify current password
    let password_hash = crate::auth::get_password_hash(
        &state.service_context.db_pool,
        &current_user.member.email
    ).await.ok().flatten();

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
            </div>"#.to_string()
        );
    }

    // Hash new password and update
    use argon2::{Argon2, PasswordHasher};
    use argon2::password_hash::{SaltString, rand_core::OsRng};

    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();

    let new_hash = match argon2.hash_password(form.new_password.as_bytes(), &salt) {
        Ok(hash) => hash.to_string(),
        Err(_) => {
            return axum::response::Html(
                r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">
                    Failed to update password
                </div>"#.to_string()
            );
        }
    };

    // Update password in database
    let result = sqlx::query("UPDATE members SET password_hash = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(&new_hash)
        .bind(current_user.member.id.to_string())
        .execute(&state.service_context.db_pool)
        .await;

    match result {
        Ok(_) => axum::response::Html(
            r#"<div class="p-3 bg-green-50 text-green-800 rounded-md text-sm">
                Password updated successfully!
            </div>"#.to_string()
        ),
        Err(_) => axum::response::Html(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">
                Failed to update password
            </div>"#.to_string()
        ),
    }
}
